# ClipDeck core

The clipboard-history, pinboard, settings and storage library used by
[ClipDeck](https://github.com/mhosseinab/clipdeck), a native Linux clipboard manager.
It includes SQLite/SQLCipher storage, classification, search, hooks and sync logic.

For the desktop application, install `clipdeck-app` or use the Snap or Debian
packages linked from the project repository.

The crate builds SQLCipher and OpenSSL from source and requires a C build
toolchain, `make` and Perl. Integration tests use fixtures and application assets
in the full repository; run `bash scripts/check.sh linux-core` from a checkout.
The registry package includes the library and its inline unit tests.

Licensed under MIT. See `LICENSE`.
