# pSMS (Papamo SMS)

pSMS is an MIT-licensed, self-hosted message archive written in Rust. Its REST API accepts encrypted uploads. iPhone Shortcuts capture is available today, and Android capture is planned. Each instance hosts one local archive with bundled SQLite. A sync webhook can write changes to a database of your choice through a compatible receiver.

[Download pSMS](https://www.papamo.dev/sms) · [Setup guide, REST API and webhook contract](https://www.papamo.dev/docs/papamo-sms)

## Run

Build from source with the commands below, or keep both release binaries together. Then run `psms init`. The daemon generates its credentials and runs in the background. Use `psms status`, `psms desktop`, `psms settings` and `psms stop`. A foreground server is available with `psms serve`. See `psms --help` for options.

```sh
cargo build --release --locked
cargo build --release --locked --manifest-path desktop-ui/Cargo.toml --target-dir target
```

Desktop builds use [GPUI Kit](https://gpui-kit.com/docs/installation). A headless server only needs the first binary.

| Environment variable | Default | Purpose |
| --- | --- | --- |
| PSMS_BIND | 127.0.0.1:47832, fallback to available port | HTTP listener behind the proxy |
| PSMS_TOKEN | generated locally | Single-archive upload credential |
| PSMS_DB_PATH | Per-user application data | Override with a SQLite file path in an existing directory |
| PSMS_API_PREFIX | /v1 | API path prefix |

## Phone setup

The default archive is `~/Library/Application Support/papamo-sms/papamo.sqlite3` on macOS, `%LOCALAPPDATA%\papamo-sms\papamo.sqlite3` on Windows, and `$XDG_DATA_HOME/papamo-sms/papamo.sqlite3` on Linux (normally `~/.local/share/papamo-sms/papamo.sqlite3`). If upgrading from the working-directory default, set PSMS_DB_PATH to the absolute path of your existing archive.

Run `psms desktop` for the native inbox. Owner and upload tokens are generated in the private archive by default. When providing an explicit PSMS_TOKEN, set a distinct PSMS_OWNER_TOKEN for owner HTTP access, or omit it for upload-only HTTP. Protect the database and backups at the OS level.

### Encrypted upload contract

POST JSON containing `id`, `sender`, and `body` to the configured prefix plus `/messages` with the bearer upload token. The sender field contains the provider marker. Encrypt the actual sender and text together on the device.

| Provider | Sender marker | Body |
| --- | --- | --- |
| Actions | `papamo:encrypted:v1` | Base64 of the 12-byte nonce, AES-GCM ciphertext and 16-byte tag |
| OpenPGP | `papamo:openpgp:v1` | ASCII-armored encrypted PGP message |

The server checks the envelope format. Only the key holder can verify the plaintext. Actions users can decrypt locally in the native desktop viewer. OpenPGP users can open the armored text in their PGP app. Owners retain access to earlier plaintext records. New plaintext uploads return 422. The 64 KiB request limit includes encoding overhead.

New uploads return 201. Identical retries return 200. Conflicting retries return 409. Owner endpoints support listing messages with cursor pagination, reading a message, patching its starred flag and deleting it. Starring preserves the original ciphertext for retries.

### Prepare a shortcut

For the CLI workflow run `psms onboard --help`. Use the [Actions template](examples/Papamo%20SMS%20Encrypted.shortcut). To prepare a private copy, set PSMS_TOKEN to the instance upload token and run the following on a trusted Mac with Python 3.10+. The new directory includes a recovery key. Supply PSMS_ARCHIVE_KEY to reuse an existing key. The person generating the shortcut can extract both secrets.

```sh
python3 examples/personalize.py ~/papamo-private --endpoint https://YOUR-HOST/v1/messages
cd ~/papamo-private
shortcuts sign --mode anyone --input Papamo_SMS_Encrypted_Private.shortcut --output Papamo_SMS_Encrypted_Ready.shortcut
```

Share a personalized signed copy privately with its owner. Keep the recovery key on trusted devices. On iOS 27, enable the Message trigger on the receiving iPhone and test a phrase containing a space. Set PSMS_ACTIONS_SHORTCUT_URL or PSMS_OPENPGP_SHORTCUT_URL to the signed HTTPS download links you want shown to the authenticated owner. Operators supply and test their own OpenPGP shortcut. See the [handoff and screenshot list](HANDOFF.md).


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


### Desktop installation

The Windows x64 installer adds pSMS to your desktop, Start menu, and Installed Apps for the current user. Close the desktop window before upgrading. On Debian or Ubuntu install the matching local package with `sudo apt install ./psms-linux-x64.deb`. On Fedora use `sudo dnf install ./psms-linux-x64.rpm`. Substitute `arm64` on 64-bit ARM. Both packages register pSMS in the applications menu and request a desktop shortcut at your next graphical login or first launch. Desktop environments control whether desktop icons are displayed and may ask you to allow launching. Portable archives remain available for all platforms.

## Contacts, access tokens and webhooks

Use a separate archive database and credentials for each recipient. The new encrypted shortcut attaches a private contact ID. The sender and body remain inside the encrypted envelope. Repeat senders are recognizable within a shortcut key. A new key starts a new contact index. Older encrypted uploads without an ID remain readable and do not create a contact.

To search, run `psms contact-id --key-file archive-recovery-key.txt` on the owner's device and enter the exact sender tag or phone number. Use the resulting ID with `GET /v1/contacts?contact_id=ID`. Owner routes also include `GET /v1/contacts/ID?include=messages` and `GET /v1/contacts/ID/messages`. Expansion includes up to five encrypted messages per contact. Follow the returned cursor on the messages endpoint for more.

In desktop Settings, create an Owner API token or a Browser autofill token, choose an expiration and click **Copy token**. Rotation replaces the selected token immediately. Revocation disables it. Existing installation administrator credentials remain separately managed. Browser tokens can only fetch recent encrypted Actions envelopes. Use separate tokens per webhook destination. Event delivery credentials must be available to the sending daemon, so protect archive files and backups.

Create HTTPS destinations under Settings, or use `POST /v1/webhooks` with an owner bearer token and a JSON object containing `url`. List with GET, rotate with `POST /v1/webhooks/ID/rotate`, and remove with `DELETE /v1/webhooks/ID`. Creation and rotation return the new token once. Each destination receives a JSON object with delivery_id, event and message_id. Handle repeated deliveries idempotently. Message content and sender names are excluded.

## Browser autofill

Download the Chrome or Firefox extension ZIP from this release. For Chrome, unzip it, open chrome://extensions, enable Developer mode, and choose Load unpacked. For Firefox testing, open about:debugging, choose This Firefox and Load Temporary Add-on, and select the ZIP. Firefox temporary installation lasts until the browser restarts.

Open the extension to detect pSMS on the default local port or enter the full API address for a custom port or HTTPS server. Enter a **Browser autofill** token from pSMS Settings and the private Actions recovery key. They stay in extension session storage until you lock or restart the browser. Enable autofill individually on each sign-in site.

Focus the verification-code field. A recent SMS ending in a line such as `@example.com #123456` can show **Autofill from GLOBE** and **123 456**. Click the suggestion to fill the field. The extension decrypts on this device, verifies the exact HTTPS hostname and ignores unbound SMS and messages over five minutes old. The sender label comes from the message and does not certify the sender.

Settings also lets you change the appearance and sync interval. New installations sync every six hours when capacity allows. The desktop starts with the system appearance and remembers a manual change. Environment variables use the `PSMS_` prefix. Previous `PAPAMO_` names remain supported when their replacements are unset.

## Connect Claude or Codex through MCP

Start your archive, put the pSMS executable on PATH, and register its stdio server with your client:

    codex mcp add psms -- psms mcp
    claude mcp add --transport stdio psms -- psms mcp

For a remote archive, append `--server https://YOUR-HOST/v1 --token-file /private/owner-token.txt` to the pSMS command. Add `--key-file /private/archive-recovery-key.txt` only when you want the client to receive decrypted Actions messages. The key stays on the machine running pSMS MCP. Your AI client receives matching plaintext when you enable decryption. Without the key, the server returns metadata.

Tools include `search_messages` (text, sender, date range, starred status and cursor), `get_message` (UUID), and `list_contacts` (opaque contact ID and optional message expansion). Follow the search cursor even when a filtered page is empty.
