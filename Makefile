# kappfinder-rs — build and installation
#
#   make                              build the release binary
#   sudo make install                 install for everyone under /usr/local
#   make install PREFIX=$HOME/.local  install for the current user only
#   make install-launcher             launcher + icon only, no build
#   make uninstall                    remove everything this installed
#
# Packagers: DESTDIR is honoured throughout, and no cache-refresh commands
# run when it is set.

PREFIX  ?= /usr/local
DESTDIR ?=

BINDIR  ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
APPDIR  ?= $(DATADIR)/applications
ICONDIR ?= $(DATADIR)/icons/hicolor/scalable/apps

CARGO ?= cargo
BIN   := target/release/kappfinder-rs

.PHONY: all build check validate install install-bin install-launcher uninstall clean

all: build

build:
	$(CARGO) build --release --locked

check:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test

# desktop-file-validate exits 0 for warnings and hints, so treat any output
# at all as a failure — the same bar CI holds generated entries to.
validate:
	@out=$$(desktop-file-validate desktop/kappfinder-rs.desktop 2>&1 || true); \
	if [ -n "$$out" ]; then \
	  echo "desktop-file-validate reported problems:"; \
	  echo "$$out"; \
	  exit 1; \
	fi; \
	echo "desktop/kappfinder-rs.desktop is valid"

install: install-bin install-launcher

install-bin: build
	install -Dm755 $(BIN) $(DESTDIR)$(BINDIR)/kappfinder-rs

# The committed .desktop invokes `kappfinder-rs` by name, which only works if
# the binary is on the $PATH of the *desktop session* — which is not the same
# as a login shell's, and frequently lacks ~/.local/bin. The installed copy is
# therefore rewritten to the absolute path the binary lands at.
install-launcher:
	install -d $(DESTDIR)$(APPDIR)
	sed -e 's|sh -c "kappfinder-rs |sh -c "$(BINDIR)/kappfinder-rs |g' \
	    -e 's|^TryExec=kappfinder-rs$$|TryExec=$(BINDIR)/kappfinder-rs|' \
	    desktop/kappfinder-rs.desktop > $(DESTDIR)$(APPDIR)/kappfinder-rs.desktop
	chmod 644 $(DESTDIR)$(APPDIR)/kappfinder-rs.desktop
	install -Dm644 desktop/kappfinder-rs.svg $(DESTDIR)$(ICONDIR)/kappfinder-rs.svg
	@if [ -z "$(DESTDIR)" ]; then \
	  update-desktop-database "$(APPDIR)" 2>/dev/null || true; \
	  gtk-update-icon-cache -qtf "$(DATADIR)/icons/hicolor" 2>/dev/null || true; \
	fi

# Menu entries the tool generated are *not* removed here: they belong to the
# user, not to this install. Run `kappfinder-rs remove` first if you want
# them gone, because only the binary knows which files are its own.
uninstall:
	rm -f $(DESTDIR)$(BINDIR)/kappfinder-rs
	rm -f $(DESTDIR)$(APPDIR)/kappfinder-rs.desktop
	rm -f $(DESTDIR)$(ICONDIR)/kappfinder-rs.svg
	@if [ -z "$(DESTDIR)" ]; then \
	  update-desktop-database "$(APPDIR)" 2>/dev/null || true; \
	  gtk-update-icon-cache -qtf "$(DATADIR)/icons/hicolor" 2>/dev/null || true; \
	fi

clean:
	$(CARGO) clean
