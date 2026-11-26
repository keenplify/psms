# Hosted browser inbox

The GPUI Kit transcript compiles to WebAssembly and runs in the hosted inbox at https://www.papamo.dev/sms/inbox. The Papamo site supplies semantic HTML navigation, keyboard controls, local recovery-key handling, a screen-reader transcript, and a text view for browsers without compatible graphics. The browser calls the same-origin portal, which verifies the Papamo session before reading the hosted account.

To regenerate the deployed assets, install the Rust nightly toolchain dated 2026-11-26 with its wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.130. Run the following from the pSMS repository with the site checkout beside it.

    python web/inbox/build.py ../papamo/public/psms-inbox

The generated phone workflow contains fixed-length placeholders. The browser replaces them with the account's upload credential and a locally generated recovery key when the user requests pairing files.
