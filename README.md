# pSMS (Papamo SMS)

An open-source Rust service that archives incoming texts forwarded by an iPhone Shortcut. The starter supports a single trusted archive per instance. SQLite is bundled.

## Run

Build from source with the commands below, or keep both release binaries together. Then run `psms init`. The daemon generates its credentials and runs in the background. Use `psms status`, `psms desktop`, `psms settings` and `psms stop`. A foreground server is available with `psms serve`. See `psms --help` for options.

```sh
cargo build --release --locked
cargo build --release --locked --manifest-path desktop-ui/Cargo.toml --target-dir target
```

Desktop builds use [GPUI Kit](https://gpui-kit.com/docs/installation). A headless server only needs the first binary.

| Environment variable | Default | Purpose |
| --- | --- | --- |
| PAPAMO_BIND | 127.0.0.1:47832, fallback to available port | HTTP listener behind the proxy |
| PAPAMO_TOKEN | generated locally | Single-archive upload credential |
| PAPAMO_DB_PATH | Per-user application data | Override with a SQLite file path in an existing directory |
| PAPAMO_API_PREFIX | /v1 | API path prefix |

## Phone setup

The default archive is `~/Library/Application Support/papamo-sms/papamo.sqlite3` on macOS, `%LOCALAPPDATA%\papamo-sms\papamo.sqlite3` on Windows, and `$XDG_DATA_HOME/papamo-sms/papamo.sqlite3` on Linux (normally `~/.local/share/papamo-sms/papamo.sqlite3`). If upgrading from the working-directory default, set PAPAMO_DB_PATH to the absolute path of your existing archive.

Run `psms desktop` for the native inbox. Owner and upload tokens are generated in the private archive by default. When providing an explicit PAPAMO_TOKEN, set a distinct PAPAMO_OWNER_TOKEN for owner HTTP access, or omit it for upload-only HTTP. Protect the database and backups at the OS level.

### Encrypted upload contract

POST JSON containing `id`, `sender`, and `body` to the configured prefix plus `/messages` with the bearer upload token. The sender field contains the provider marker. Encrypt the actual sender and text together on the device.

| Provider | Sender marker | Body |
| --- | --- | --- |
| Actions | `papamo:encrypted:v1` | Base64 of the 12-byte nonce, AES-GCM ciphertext and 16-byte tag |
| OpenPGP | `papamo:openpgp:v1` | ASCII-armored encrypted PGP message |

The server checks the envelope format. Only the key holder can verify the plaintext. Actions users can decrypt locally in the native desktop viewer. OpenPGP users can open the armored text in their PGP app. Owners retain access to earlier plaintext records. New plaintext uploads return 422. The 64 KiB request limit includes encoding overhead.

New uploads return 201. Identical retries return 200. Conflicting retries return 409. Owner endpoints support listing messages with cursor pagination, reading a message, patching its starred flag and deleting it. Starring preserves the original ciphertext for retries.

### Prepare a shortcut

For the CLI workflow run `psms onboard --help`. Use the [Actions template](examples/Papamo%20SMS%20Encrypted.shortcut). To prepare a private copy, set PAPAMO_TOKEN to the instance upload token and run the following on a trusted Mac with Python 3.10+. The new directory includes a recovery key. Supply PAPAMO_ARCHIVE_KEY to reuse an existing key. The person generating the shortcut can extract both secrets.

```sh
python3 examples/personalize.py ~/papamo-private --endpoint https://YOUR-HOST/v1/messages
cd ~/papamo-private
shortcuts sign --mode anyone --input Papamo_SMS_Encrypted_Private.shortcut --output Papamo_SMS_Encrypted_Ready.shortcut
```

Share a personalized signed copy privately with its owner. Keep the recovery key on trusted devices. On iOS 27, enable the Message trigger on the receiving iPhone and test a phrase containing a space. Set PAPAMO_ACTIONS_SHORTCUT_URL or PAPAMO_OPENPGP_SHORTCUT_URL to the signed HTTPS download links you want shown to the authenticated owner. Operators supply and test their own OpenPGP shortcut. See the [handoff and screenshot list](HANDOFF.md).


## Remote access

For a temporary Cloudflare URL run `psms tunnel quick`. For a named tunnel run `psms tunnel cloudflare --token-file PATH` after configuring a published hostname pointed at the address reported by `psms status`. Both commands require cloudflared on PATH. Other outbound tunnel clients can forward to the same origin. Verify bearer-header forwarding and encrypted POST delivery before using any new provider. A reachable HTTPS reverse proxy can go directly to pSMS. Fixed tunnel origins should use a fixed `--bind`. Running public tunnels and Apple signing are operator actions.

## Regional sync

Each archive accepts uploads locally and can sync its encrypted messages to a shared receiver. [Pair two archives and check the sync schedule](SYNC.md).

## Builds

### Developer hooks

Install [Lefthook](https://github.com/evilmartians/lefthook) and run the following once per checkout:

```sh
rustup component add rustfmt clippy
lefthook install
```

Before each commit, the hook checks Rust formatting and runs Clippy across all Cargo targets with warnings treated as errors. Run `cargo fmt --all` to apply formatting, or `lefthook run pre-commit` to run both checks manually.

Run `cargo build --locked --release` on the target system. CI builds and tests Linux x86-64 and ARM64 (GNU), Windows x86-64 (MSVC), and macOS Intel and Apple silicon. Each job uploads its native executable. Linux binaries inherit the build runner's system library requirements.

## License

MIT.

