# ClipDeck for Linux

Native clipboard history with search, pinboards, text and image clips, a paste
stack, and optional encrypted storage. The executable is named `clipdeck`.

```sh
cargo install clipdeck-app --locked
clipdeck run
```

Building requires Rust 1.89 or newer, a C build toolchain, `make`, Perl,
`pkg-config` and the D-Bus development library (`libdbus-1-dev` on Ubuntu).
SQLCipher and OpenSSL are built from source. A graphical Linux session is
required to run the app. Encrypted history also needs Secret Service and
`secret-tool` (`libsecret-tools` on Ubuntu).

Cargo installs the executable without desktop entries or icons in system
directories. For desktop package installation, see the
[Snap and Debian downloads](https://github.com/mhosseinab/clipdeck#install).

On GNOME Wayland, add a custom keyboard shortcut for
`/home/YOUR_USER/.cargo/bin/clipdeck toggle`, adjusting the path if `CARGO_HOME`
is customized. ClipDeck must already be running. Selecting a clip copies it;
paste it into the destination app yourself.

[User guide](https://github.com/mhosseinab/clipdeck/blob/main/docs/usage.md)
and [source](https://github.com/mhosseinab/clipdeck).

Licensed under MIT. The bundled Noto font and text-rendering libraries have
additional notices in `assets/fonts/OFL-NotoSansArabic.txt` and
`THIRD-PARTY-TEXT-LICENSES.txt`.
