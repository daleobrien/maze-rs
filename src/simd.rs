//! Hand-vectorised kernels, using NEON on aarch64 (always present there) and
//! a plain scalar loop everywhere else. Each one returns exactly what its
//! scalar version does, so output never depends on the target.

const ADLER_BASE: u32 = 65_521;

/// The most bytes that can be summed before the Adler-32 sums can overflow
/// a `u32`.
const ADLER_NMAX: usize = 5552;

/// Adler-32 of `data`, as zlib computes it.
#[must_use]
pub fn adler32(data: &[u8]) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        adler32_neon(data)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        adler32_scalar(1, 0, data)
    }
}

/// Adler-32 one byte at a time, continuing from the sums `a` and `b`.
fn adler32_scalar(mut a: u32, mut b: u32, data: &[u8]) -> u32 {
    for chunk in data.chunks(ADLER_NMAX) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_BASE;
        b %= ADLER_BASE;
    }
    (b << 16) | a
}

#[cfg(target_arch = "aarch64")]
fn adler32_neon(data: &[u8]) -> u32 {
    use std::arch::aarch64::{
        uint16x8_t, uint32x4_t, vaddq_u32, vaddvq_u32, vaddw_high_u8, vaddw_u8, vdupq_n_u16, vdupq_n_u32,
        vget_low_u16, vget_low_u8, vld1q_u16, vld1q_u8, vmlal_high_u16, vmlal_u16, vmull_u16,
        vpadalq_u16, vpaddlq_u8,
    };

    // A block of 16 bytes adds each byte to `a`, and to `b` that byte times
    // its distance from the end of the run. The per-column byte totals are
    // kept in u16 lanes, which hold 256 blocks of 0xff before overflowing.
    const BLOCK: usize = 16;
    const RUN: usize = 256 * BLOCK;
    const WEIGHTS: [u16; 16] = [16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];

    let (mut a, mut b) = (1u32, 0u32);

    let sum_run = |run: &[u8], a: &mut u32, b: &mut u32| {
        // SAFETY: NEON is part of the aarch64 baseline, and every load reads
        // 16 bytes from a chunk of exactly 16 (or a 16-element array).
        unsafe {
            let (mut v_a, mut v_b): (uint32x4_t, uint32x4_t) = (vdupq_n_u32(0), vdupq_n_u32(0));
            let (mut col_lo, mut col_hi): (uint16x8_t, uint16x8_t) = (vdupq_n_u16(0), vdupq_n_u16(0));
            for block in run.as_chunks::<BLOCK>().0 {
                let bytes = vld1q_u8(block.as_ptr());
                // every earlier block's bytes count once more in `b`
                v_b = vaddq_u32(v_b, v_a);
                v_a = vpadalq_u16(v_a, vpaddlq_u8(bytes));
                col_lo = vaddw_u8(col_lo, vget_low_u8(bytes));
                col_hi = vaddw_high_u8(col_hi, bytes);
            }
            let w_lo = vld1q_u16(WEIGHTS.as_ptr());
            let w_hi = vld1q_u16(WEIGHTS.as_ptr().add(8));
            let mut weighted = vmull_u16(vget_low_u16(col_lo), vget_low_u16(w_lo));
            weighted = vmlal_high_u16(weighted, col_lo, w_lo);
            weighted = vmlal_u16(weighted, vget_low_u16(col_hi), vget_low_u16(w_hi));
            weighted = vmlal_high_u16(weighted, col_hi, w_hi);
            let weighted = u64::from(vaddvq_u32(weighted));

            let n = run.len() as u64;
            let new_b = u64::from(*b) + u64::from(*a) * n + 16 * u64::from(vaddvq_u32(v_b)) + weighted;
            let new_a = u64::from(*a) + u64::from(vaddvq_u32(v_a));
            *a = (new_a % u64::from(ADLER_BASE)) as u32;
            *b = (new_b % u64::from(ADLER_BASE)) as u32;
        }
    };

    let (runs, rest) = data.as_chunks::<RUN>();
    for run in runs {
        sum_run(run, &mut a, &mut b);
    }
    let (whole, tail) = rest.split_at(rest.len() / BLOCK * BLOCK);
    if !whole.is_empty() {
        sum_run(whole, &mut a, &mut b);
    }
    adler32_scalar(a, b, tail)
}

/// Append the open edges of row `row` (not its first cell) to `edges`,
/// starting at `kept`, and return the new end of the list.
///
/// Each cell `x` has an edge to its north neighbour (in `above`) and one to
/// its west neighbour, numbered `cell * 2` and `cell * 2 + 1` with cells
/// numbered from `first`; an edge is kept unless a crossing is at either end.
/// `edges` must have room for 4 entries past the last one written.
pub(crate) fn push_row_edges(above: &[u8], row: &[u8], first: usize, edges: &mut [u32], kept: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        push_row_edges_neon(above, row, first, edges, kept)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        push_row_edges_scalar(above, row, first, edges, kept, 1)
    }
}

/// Whether a cell is not a crossing, as 0 or 1.
fn open(cell: u8) -> usize {
    usize::from((cell != crate::maze::CROSS_NS) & (cell != crate::maze::CROSS_EW))
}

/// The scalar version of [`push_row_edges`], from column `from` on.
///
/// Whether a crossing is nearby is random, so rather than branch on it, each
/// candidate edge is always written and the end of the list only moves past
/// it if it is kept.
fn push_row_edges_scalar(above: &[u8], row: &[u8], first: usize, edges: &mut [u32], mut kept: usize, from: usize) -> usize {
    for x in from..row.len() {
        let id = ((first + x) * 2) as u32;
        let here = open(row[x]);
        edges[kept] = id;
        kept += here & open(above[x]);
        edges[kept] = id + 1;
        kept += here & open(row[x - 1]);
    }
    kept
}

/// For each 4-bit mask of kept lanes, the byte shuffle that packs those u32
/// lanes to the front.
#[cfg(target_arch = "aarch64")]
const PACK: [[u8; 16]; 16] = {
    let mut table = [[0u8; 16]; 16];
    let mut mask = 0;
    while mask < 16 {
        let mut out = 0;
        let mut lane = 0;
        while lane < 4 {
            if mask & (1 << lane) != 0 {
                let mut byte = 0;
                while byte < 4 {
                    table[mask][out * 4 + byte] = (lane * 4 + byte) as u8;
                    byte += 1;
                }
                out += 1;
            }
            lane += 1;
        }
        mask += 1;
    }
    table
};

/// [`push_row_edges`] eight cells at a time: the kept edges of each group of
/// four slots are packed with a table lookup and stored together, so the end
/// of the list moves once per four edges rather than once per edge.
#[cfg(target_arch = "aarch64")]
fn push_row_edges_neon(above: &[u8], row: &[u8], first: usize, edges: &mut [u32], mut kept: usize) -> usize {
    use std::arch::aarch64::{
        vaddq_u32, vandq_u8, vcombine_u8, vceq_u8, vdup_n_u8, vdupq_n_u32, vld1_u8, vld1q_u32, vld1q_u8, vmvn_u8,
        vorr_u8, vpaddlq_u16, vpaddlq_u8, vqtbl1q_u8, vreinterpretq_u32_u8, vreinterpretq_u8_u32, vst1q_u32,
        vzip1_u8, vzip2_u8, uint8x8_t,
    };
    use crate::maze::{CROSS_EW, CROSS_NS};

    const LANE_BITS: [u8; 16] = [1, 2, 4, 8, 1, 2, 4, 8, 1, 2, 4, 8, 1, 2, 4, 8];
    const IOTA: [u32; 4] = [0, 1, 2, 3];

    let width = row.len();
    assert!(above.len() == width, "rows have the same width");
    let mut x = 1;
    // SAFETY: NEON is part of the aarch64 baseline; each 8-byte load reads
    // columns x-1..x+8 of a row, all below `width`, and each store goes to a
    // checked 4-entry slice of `edges`.
    unsafe {
        let open = |cells: uint8x8_t| {
            let crossing = vorr_u8(vceq_u8(cells, vdup_n_u8(CROSS_NS)), vceq_u8(cells, vdup_n_u8(CROSS_EW)));
            vmvn_u8(crossing)
        };
        let lane_bits = vld1q_u8(LANE_BITS.as_ptr());
        let iota = vld1q_u32(IOTA.as_ptr());
        while x + 8 <= width {
            let here = open(vld1_u8(row.as_ptr().add(x)));
            let north = vandq_u8_d(here, open(vld1_u8(above.as_ptr().add(x))));
            let west = vandq_u8_d(here, open(vld1_u8(row.as_ptr().add(x - 1))));
            // slots in list order: north then west, cell by cell
            let slots = vcombine_u8(vzip1_u8(north, west), vzip2_u8(north, west));
            let masks = vpaddlq_u16(vpaddlq_u8(vandq_u8(slots, lane_bits)));
            let mut groups = [0u32; 4];
            vst1q_u32(groups.as_mut_ptr(), masks);

            let base = ((first + x) * 2) as u32;
            for (g, &mask) in groups.iter().enumerate() {
                let ids = vaddq_u32(vdupq_n_u32(base + 4 * g as u32), iota);
                let packed = vqtbl1q_u8(vreinterpretq_u8_u32(ids), vld1q_u8(PACK[mask as usize].as_ptr()));
                let dst = &mut edges[kept..kept + 4];
                vst1q_u32(dst.as_mut_ptr(), vreinterpretq_u32_u8(packed));
                kept += mask.count_ones() as usize;
            }
            x += 8;
        }
    }
    push_row_edges_scalar(above, row, first, edges, kept, x)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn vandq_u8_d(a: std::arch::aarch64::uint8x8_t, b: std::arch::aarch64::uint8x8_t) -> std::arch::aarch64::uint8x8_t {
    std::arch::aarch64::vand_u8(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler32_matches_scalar() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let data: Vec<u8> = (0..70_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect();
        for len in (0..300).chain([4095, 4096, 4097, 5551, 5552, 5553, 8192, 69_999, 70_000]) {
            assert_eq!(adler32(&data[..len]), adler32_scalar(1, 0, &data[..len]), "len {len}");
            // all 0xff is the worst case for overflow
            let ones = vec![0xffu8; len];
            assert_eq!(adler32(&ones), adler32_scalar(1, 0, &ones), "0xff len {len}");
        }
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn row_edges_match_scalar() {
        use crate::maze::{CROSS_EW, CROSS_NS};
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut cell = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            [0, 3, 5, CROSS_NS, CROSS_EW, 15][(state % 6) as usize]
        };
        for width in 1..70 {
            for _ in 0..20 {
                let above: Vec<u8> = (0..width).map(|_| cell()).collect();
                let row: Vec<u8> = (0..width).map(|_| cell()).collect();
                let (mut want, mut got) = (vec![0u32; 2 * width + 8], vec![0u32; 2 * width + 8]);
                let n = push_row_edges_scalar(&above, &row, 1000, &mut want, 3, 1);
                let m = push_row_edges(&above, &row, 1000, &mut got, 3);
                assert_eq!((m, &got[..m]), (n, &want[..n]), "width {width}");
            }
        }
    }
}
