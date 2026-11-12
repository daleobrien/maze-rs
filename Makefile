# Convenience wrapper around cargo and the maze CLI.
# Run `make` (or `make help`) for the list. W=... and H=... set the maze size
# for `make text`, `make svg` and `make pdf`, ID=... fixes the maze id (a
# random maze otherwise), INK=1 draws `make svg` and `make pdf` by hand in ink
# (-I), OPTS=... passes more options (e.g. OPTS="--corners bevel --frame") and
# BENCH=... filters `make bench` (e.g. BENCH=render).

.DEFAULT_GOAL := help
.PHONY: help check build debug test bench run text svg pdf ink install fmt clippy lint clean

W ?= 21
H ?= 30
ID ?=
INK ?=
OPTS ?=
BENCH ?=
OUT ?= out

# cargo run rather than target/release/maze, since ~/.cargo/config.toml may
# put the build somewhere else.
MAZE := cargo run --release --quiet --
ID_ARG := $(if $(ID),-i $(ID))
INK_ARG := $(if $(INK),-I)

help:
	@echo "maze — common tasks"
	@echo
	@echo "  make check      cargo check"
	@echo "  make build      cargo build --release"
	@echo "  make test       cargo test --release, including the snapshot hashes"
	@echo "  make bench      cargo bench (BENCH=render to run just the renderers)"
	@echo "  make run        cargo run --release — prints the usage"
	@echo "  make text       print a W x H maze (ID=... for a fixed one)"
	@echo "  make svg        write a W x H maze and its solution to $(OUT)/maze.svg and maze-solved.svg"
	@echo "  make pdf        write a W x H maze to $(OUT)/maze.pdf (OPTS=\"-s out/solved.pdf\" for its solution)"
	@echo "                 (INK=1 draws svg and pdf by hand, in ink; OPTS=\"--frame ...\" adds style options)"
	@echo "  make ink        hand-drawn svg, solution and pdf of one maze in $(OUT)/maze-ink.*"
	@echo "  make install    cargo install --path . — puts maze on your PATH"
	@echo "  make lint       cargo fmt --check, then clippy"

check:
	cargo check --all-targets
build:
	cargo build --release
debug:
	cargo build
test:
	cargo test --release
bench:
	cargo bench --bench maze $(if $(BENCH),-- $(BENCH))
run:
	cargo run --release
text:
	$(MAZE) text -W $(W) -H $(H) $(ID_ARG) $(OPTS)
svg:
	@mkdir -p $(OUT)
	$(MAZE) svg $(OUT)/maze.svg -s $(OUT)/maze-solved.svg -W $(W) -H $(H) $(ID_ARG) $(INK_ARG) $(OPTS)
	@echo "Wrote $(OUT)/maze.svg and $(OUT)/maze-solved.svg"
pdf:
	@mkdir -p $(OUT)
	$(MAZE) pdf $(OUT)/maze.pdf -W $(W) -H $(H) $(ID_ARG) $(INK_ARG) $(OPTS)
	@echo "Wrote $(OUT)/maze.pdf"
ink:
	@mkdir -p $(OUT)
	$(eval INK_ID := $(or $(ID),$(shell date +%s)))
	$(MAZE) svg $(OUT)/maze-ink.svg -s $(OUT)/maze-ink-solved.svg -W $(W) -H $(H) -i $(INK_ID) -I $(OPTS)
	$(MAZE) pdf $(OUT)/maze-ink.pdf -W $(W) -H $(H) -i $(INK_ID) -I $(OPTS)
	@echo "Wrote $(OUT)/maze-ink.svg, maze-ink-solved.svg and maze-ink.pdf (maze id $(INK_ID))"
install:
	cargo install --path .
fmt:
	cargo fmt
clippy:
	cargo clippy --all-targets
lint:
	cargo fmt --check
	cargo clippy --all-targets
clean:
	cargo clean
