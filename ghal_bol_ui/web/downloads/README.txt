Linux desktop bundle for https://ghalbol.com

Filename (required):
  ghal-bol-linux-x64.tar.gz

Served at:
  https://ghalbol.com/downloads/ghal-bol-linux-x64.tar.gz

Must exist BEFORE:  flutter build web --release
Then deploy:        ./scripts/deploy_web_firebase.sh  (from repo root)

--- Build the Linux app (x86_64 host) ---

From repo root:
  ./scripts/sync_ghal_bol_native_for_flutter.sh
  cd ghal_bol_ui
  flutter build linux --release

--- Create the tarball ---

From ghal_bol_ui/ (extracts as folder "bundle"):

  mkdir -p web/downloads
  tar -czvf web/downloads/ghal-bol-linux-x64.tar.gz \
    -C build/linux/x64/release \
    bundle

Optional — top-level folder ghal-bol-linux-x64/ after extract:

  cd build/linux/x64/release
  cp -a bundle ghal-bol-linux-x64
  tar -czvf ../../../../web/downloads/ghal-bol-linux-x64.tar.gz ghal-bol-linux-x64
  rm -rf ghal-bol-linux-x64

Full site docs: docs/WEB_SITE.md
