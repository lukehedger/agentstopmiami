EXT_DIR := $(HOME)/.pi/agent/extensions
EXT     := miami-report.ts

.PHONY: install bin ext uninstall test run fake

## install: build + install the binaries and the pi extension
install: bin ext
	@echo
	@echo "run 'miami' in a pane; restart your pi panes to load the extension"

## bin: install miami and miami-tail to ~/.cargo/bin
bin:
	cargo install --path . --quiet
	@echo "installed: $$(command -v miami)"

## ext: copy the reporting extension into pi's global extension dir
ext:
	@mkdir -p $(EXT_DIR)
	cp extension/$(EXT) $(EXT_DIR)/$(EXT)
	@echo "installed: $(EXT_DIR)/$(EXT)"

## uninstall: remove binaries and extension
uninstall:
	-cargo uninstall miami
	-rm -f $(EXT_DIR)/$(EXT)

## test: run the suite
test:
	cargo test

## run: build and run the dashboard from the working tree
run:
	cargo run --release

## fake: emit fake agents at the socket (needs miami running)
fake:
	./scripts/fake-agents.py -n 5
