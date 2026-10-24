.PHONY: all precommit lint format test coverage clean

all: precommit

# ─── Pre-commit (read-only checks) ───────────────────────────────────

precommit:
	bash scripts/precommit.sh

# ─── Lint (read-only, used by precommit) ─────────────────────────────

lint: check clippy fmt-check file-length

check:
	RUSTFLAGS="-D warnings" cargo check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings -D clippy::too_many_lines

# ─── File/module size caps (per-file; clippy owns the per-fn cap) ─────
file-length:
	bash scripts/check-file-length.sh

fmt-check:
	cargo fmt --check

# ─── Format (write-mode) ─────────────────────────────────────────────

format:
	cargo fmt

test:
	RUSTFLAGS="-D warnings" cargo test --locked --workspace --tests

ci: lint test

# ─── Coverage report (no percentage threshold) ─────────────────────────

coverage:
	# No coverage-artifact reuse: stale hashed/instrumented objects have
	# wedged llvm-cov export before. Every run starts from a clean slate.
	rm -rf target/llvm-cov-target coverage && mkdir -p coverage && \
	if [ -x /usr/bin/llvm-cov ] && [ -x /usr/bin/llvm-profdata ]; then \
		export LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata; \
	fi && \
	CARGO_TARGET_DIR=target RUSTFLAGS="-D warnings" \
	cargo llvm-cov --lcov --output-path coverage/lcov.info \
		--ignore-filename-regex 'src/main\.rs|src/seed\.rs' \
		--workspace
# ─── Cleanup ──────────────────────────────────────────────────────────

clean:
	cargo clean
	rm -rf coverage/
