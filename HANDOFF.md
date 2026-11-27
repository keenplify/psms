# pSMS handoff

## Start and verify

1. Put the matching `psms` and `psms-desktop` executables in one directory. Run `psms init`. On Windows, find **pSMS** in the notification area, including its overflow menu.
2. Run `psms status` to see the selected local HTTP address. Right-click the tray icon and choose **Open** for the native GPUI Kit dashboard. **Settings** opens the inbox preferences. Close the window to leave ingestion running. **Exit** shuts down the daemon. On other platforms use `psms desktop`, `psms settings`, and `psms stop`.
3. Choose a reachable HTTPS origin. Forward an existing HTTPS reverse proxy to the reported local address, start a named Cloudflare tunnel, or run `psms tunnel quick` in a second terminal with cloudflared installed. A temporary URL changes when the connector restarts. For another provider point its outbound connector at the same HTTP origin. Verify that it passes the Authorization header and POST body. CGNAT permits this outbound setup.
4. Run `psms onboard --endpoint https://YOUR-HOST/v1/messages --output PRIVATE-NEW-FOLDER`. Keep the recovery key on a trusted device. Sign the private shortcut on macOS and import it on the phone. This step still requires Apple signing before distribution. For a second phone or an OpenPGP shortcut, match the encrypted wire format in README. The native viewer decrypts an Actions message with that message's archive key.
5. On an unlocked test phone, receive “pSMS test one”. Refresh the dashboard, enter the recovery key and select the conversation. Repeat while locked, checking for a second row before unlocking. Mac-to-iPhone library sync was observed by the owner. Background delivery, signed distribution and actual iPGMail output need device testing.
6. Verify the open/close/exit menu experience manually on the Windows desktop. Verify Linux and Apple Silicon macOS from CI artifacts. No public infrastructure or cloud account is provisioned by this checkout. The background process runs for the current session; boot-time service installation is future work.

## Mac capture handoff — iPhone and iPad

The first simulator frames show the live hosted sign-in screen in Safari. [iPhone portrait](docs/screenshots/mobile/iPhone-sign-in.png) · [iPad portrait](docs/screenshots/mobile/iPad-sign-in.png).

The Mac is the publisher’s capture workstation. The reader follows Safari, Files and Shortcuts on their own mobile device. Capture one full set on iPhone and another on iPad. Use a disposable account and synthetic messages throughout.

### Hosted pairing with the shared pSMS shortcut

Hosted accounts all use one shared, signed shortcut named **pSMS**: https://www.icloud.com/shortcuts/f1632cbef29643d19b9bb7ba28daa6aa. It holds no credentials; its single import question receives each person's private setup. In **Connect a phone**, a computer shows a QR code that opens `papamo.dev/sms/pair` on the iPhone, with the setup in the link fragment. On an iPhone, the dialog shows the same buttons directly. The phone page copies the setup and opens the iCloud link, and the user pastes the setup and taps **Add Shortcut**. Step 4 of the dialog confirms the first run and the first real text. The iPhone frames are in the Papamo site under `public/images/psms/pair-phone-*.png`, and the Mac frames under `mac-*.png`, all marked with red boxes and arrows.

To republish the shortcut after changing `examples/pSMS-Capture.shortcut`: copy it to a file named `pSMS.shortcut`, run `shortcuts sign --mode anyone`, open the signed file on a Mac, answer the setup question with the placeholder `PSMS_SETUP`, choose **Replace** for the existing template named pSMS, then use Share → Copy iCloud Link in its editor. Check the link with `https://www.icloud.com/shortcuts/api/records/<id>` (download the `shortcut` field and confirm the import question and fixes), then update `app/sms/pair/route.ts` and this file. The shortcut carries its own Message automation ("When I Get a Message Containing ' '"); on macOS it appears under Automation in the Shortcuts sidebar with an on/off switch. Turn it on for one device only, or each text is uploaded twice.

Only a change to the shortcut needs a Mac. Regenerate `examples/pSMS-Capture.shortcut`, sign it as `pSMS.shortcut`, import it on the Mac and use **Share → Copy iCloud Link**. Replace the link in the site's `app/sms/pair/route.ts` and `app/sms/inbox/shell.ts`. Phones keep the copy they installed until their owner pairs again.

### First run of a self-signed shortcut (self-hosted)

Self-hosted operators still create a personal shortcut with `psms onboard` or the desktop app and sign it on a Mac. The frames below show that flow.


Adding the shortcut is not the last step. Open it once by hand so the system asks for its permissions. Approve **Always Allow** at the network prompt, otherwise background message runs stop at it. A run without an incoming message uploads one **pSMS setup** check message instead of a text. It confirms the upload credential and archive key end to end. Shortcuts built before this change uploaded an empty message instead. The inbox, desktop and MCP show those rows as the same setup check.

| Step | Mac | iPhone | iPad |
| --- | --- | --- | --- |
| Open the downloaded file | Double-click `pSMS_Ready.shortcut` in Finder. | [Files → Downloads](docs/screenshots/setup/iphone/01-files.png) | [Files → Downloads](docs/screenshots/setup/ipad/01-files.png) |
| Add it | [Add Shortcut](docs/screenshots/setup/mac/01-add-shortcut.png) | [Add Shortcut](docs/screenshots/setup/iphone/02-add-shortcut.png) | [Add Shortcut](docs/screenshots/setup/ipad/02-add-shortcut.png) |
| Run it once | [Double-click the tile](docs/screenshots/setup/mac/02-library.png), then [press ▶](docs/screenshots/setup/mac/03-open-and-run.png) | [Tap the tile](docs/screenshots/setup/iphone/03-library.png) | [Tap the tile](docs/screenshots/setup/ipad/03-library.png) |
| Allow the Actions app | [Allow](docs/screenshots/setup/mac/04-allow-actions.png) | Allow (physical device only) | Allow (physical device only) |
| Allow the upload | [Always Allow](docs/screenshots/setup/mac/05-allow-network.png) | [Allow](docs/screenshots/setup/iphone/04-allow-connect.png) | [Allow](docs/screenshots/setup/ipad/04-allow-connect.png) |
| Check the inbox | A **pSMS setup** conversation appears. | Same | Same |

Captured on 27 Nov 2026. The Mac frames come from macOS Shortcuts with the Actions app installed. The iPhone 17 Pro and iPad Pro 11-inch (M5) frames come from iOS 26.2 simulators. All runs used the current template with a dummy upload credential and demo key, and the prompts were declined or rejected, so nothing was stored. Simulators cannot install the Actions app. Their capture copy therefore replaced **Encrypt** with **Base64 Encode**. On a physical iPhone or iPad, also expect the “run actions from Actions” prompt shown in the Mac frames. The demo shortcut is named `pSMS_Setup`; the file downloaded from the inbox is named `pSMS_Ready`.

### Launch the walkthrough

From a Mac checkout, install an iOS simulator runtime in Xcode, then run one of these commands:

```sh
bash scripts/mac-onboarding.sh iphone
bash scripts/mac-onboarding.sh ipad
```

The launcher signs the credential-free **pSMS Capture** template, opens it in Mac Shortcuts, boots an available simulator of the requested family and opens the hosted inbox in Safari. It prints the device-specific screenshot command. To pick a runtime explicitly, pass its UDID as the second argument after listing devices with `xcrun simctl list devices available`. Use `mac` for the Shortcuts import alone or `sign-only` for a signed distribution file. macOS or Apple may ask for approval. Enable the import on screen. For Safari-only captures without iCloud, prefix the command with `PSMS_SIMULATOR_WEB_ONLY=1`. On iPad, use Simulator’s Rotate Left/Right control for the landscape pass. The workflow captures the public Safari screen on both device families independently of signing. Inspect each capture before publishing it.

### Publish the reusable shortcut once

Open the signed **pSMS Capture** on the Mac. Inspect the single setup import question and confirm the default is empty or `{}`. Use the untouched template to create an iCloud sharing link, or distribute the public signed file from a trusted HTTPS page. Validate the published link in Safari on a device with a different Apple account. Paste private setup only into a recipient’s own copy after import. Personal setup includes an upload credential and encryption key. Keep personalized shortcuts and pSMS.json private. The hosted link above was republished from this template on 27 Nov 2026 with the contact-card steps: when an upload answers `contact_card: wanted`, the shortcut reads the sender's name, company, job title, phone numbers, emails and photo from Contacts, seals them with the archive key and sends them with `PUT /contacts/{id}/card`. The first card prompts for Contacts access on the iPhone. Apple documents [sharing a shortcut](https://support.apple.com/guide/shortcuts/apdf01f8c054/ios) and [import questions](https://support.apple.com/guide/shortcuts/apdf330fd3a0/ios).

### Phone-only steps to photograph

| File | User action and frame |
| --- | --- |
| 01-sign-in | Open papamo.dev/sms/inbox in Safari. Sign in with the demo account. |
| 02-first-run | Show the empty inbox and tap **Set up my iPhone** or **Connect a phone**. |
| 03-backup | Create pairing files. Save **pSMS-Recovery-Kit.pdf** to Files or write the numbered words on paper. Show the download button or a clearly marked public sample. |
| 04-private-setup | Use **Copy private phone setup** in the pairing panel. Self-hosted users can copy the contents of their private pSMS.json from their operator. Keep the clipboard contents off camera. |
| 05-install | Install Actions on the receiving device. Open the public pSMS Capture link in mobile Safari, start import, paste setup into the single import question and finish adding the shortcut. Capture the question while it is empty. |
| 06-trigger | On the receiving iPhone open Shortcuts → Automation. Enable the imported Message trigger if present. Otherwise create a personal Message automation, select a test sender or supported filter, and pass the incoming Message to **Run Shortcut → pSMS Capture**. |
| 07-enable | Select **Run Immediately** where available. Capture the actual enabled state and any first-run network permission for the test archive. |
| 08-delivery | Receive an ordinary SMS containing “pSMS test one” on a physical iPhone. Verify the synthetic sender and timestamp in the browser inbox. Repeat with the receiving iPhone locked and check from a second signed-in device before unlocking. |
| 09-restore | In a fresh test browser profile, sign in and show the empty recovery field. Enter the 12 demo words privately, then capture the restored synthetic conversation. |

On **iPad**, cover portrait and landscape Safari, Files download, account menu, recovery and reading a conversation from the paired iPhone. Capture Split View if available. If demonstrating Messages forwarded from an iPhone, record the exact device settings and separately verify whether its personal automation fires. Claim iPad capture only for a configuration tested end to end. Apple describes [forwarding iPhone texts to other devices](https://support.apple.com/en-us/102545).

Store captures under `ios-<version>/iphone/`, `ipados-<version>/portrait/` and `ipados-<version>/landscape/`. Record device, runtime, Actions version, locale, and simulator versus hardware. Capture simulator frames with `xcrun simctl io <UDID> screenshot <file>.png`. Check Shortcuts, Actions and trigger availability in each runtime; capture missing steps on physical hardware. Use a real incoming SMS for delivery evidence. Simulator APNs injection tests a different trigger. Keep recovery phrases, account passwords, raw phone configuration, real messages, and private share links out of published screenshots.


## Feature parity

AGENTS.md asks every surface to match. Web is the hosted inbox at papamo.dev/sms/inbox; Desktop is the GPUI app on a self-hosted archive; CLI and MCP are `psms` subcommands. "—" marks a feature that does not apply there. Close the remaining gaps or explain them here.

| Feature | Web | Desktop | CLI | MCP |
| --- | --- | --- | --- | --- |
| Read conversations | Yes | Yes | `manage inbox` | search and get tools |
| Unlock with the 12 words or key | Yes | Yes | key file | key at launch |
| Unlock with the recovery kit PDF | Yes | Yes | `--key-file` takes the PDF | `--key-file` takes the PDF |
| Star messages | Yes: click a bubble or ☆, filter ★ | Yes | `manage inbox` | `update_message`, star filter |
| Read and unread status | Yes (hosted) | Yes (SQLite storage) | `inbox read`, `unread`, `list --unread` | `update_message`, `read` field and `unread` filter |
| New messages without refreshing | pushed | polls the local file | — | — |
| Contact names and photos | Yes | Yes (SQLite storage) | `contact list`, `show` (photo reported, not shown) | contact cards on messages and contacts |
| Edit contacts | Yes, with photo | Yes, with photo | `contact set`, `reset` (no photo) | `set_contact`, `reset_contact` (no photo) |
| Labelled phone numbers and emails | Yes | Yes | `--phone label=value` | Yes |
| Connect a phone | QR and setup page | self-hosted setup | `onboard` | — |
| Recovery kit again | Settings | Settings | `onboard` writes a fresh kit | — |
| Appearance | Settings | Settings | `manage theme` | — |
| Browser autofill | Settings | Access tokens | `manage access` | — |
| Scoped tokens, webhooks, sync timing | — (hosted has none) | Settings | `manage` | — |
| Connected phones list | Settings | — (one upload token) | `manage access` | — |
| Hosted archive | Yes | Yes: setup's "Use my papamo.dev inbox" or Settings → Reply from anywhere; reads, stars, read status, cards and replies through the relay's computer token, refetching when the outbox claim reports a newer message | Gap | Gap |
| Reply from a conversation | Yes: sealed in the browser, queued for pSMS for Mac | Yes on Mac, through Messages | `outbox send` | `send_message` |
| Reply from anywhere (Mac at home sends) | Queues; shows waiting, sending, failed | Settings → Reply from anywhere: papamo.dev or any self-hosted URL | `outbox send`, `status` (local archive) | `send_message`, `outbox_status` |
| Connect a self-hosted archive to papamo.dev (Plus) | Reads and replies to the synced copy | Settings → papamo.dev Plus (connect, restore, disconnect) | `manage cloud connect/status/restore/disconnect` | `cloud_status` |
| Team inboxes (Plus) | Settings → Team inboxes; open a shared inbox | Gap: desktop opens self-hosted archives only | Gap | Gap |
| Managed webhooks (Plus) | Settings → Webhooks, delivery log, retry | Gap: self-hosted keeps its own webhooks in Settings | Gap | Gap |
| One-time code alerts | Browser extension (autofill) | Native notification with Copy code / Type code; Tab types the code for a minute (macOS; Windows and Linux show a plain notification and copy the code) | — | — |
| Devices-only send API (LAN, Tailscale) | — (hosted) | Settings → Your devices, send key | `manage devices`, `access create send` | — (configuration) |
| Notification for each new text | Settings → Notifications: while the tab is open and unlocked, even in the background (browser notifications, no push service yet) | Settings → Notifications (on by default); codes keep their own Copy/Type alert | — (no long-running UI) | — |
| Start a new conversation | ✎ beside the star filter: To field, then the usual composer; queued for pSMS for Mac | + beside Messages (pSMS for Mac only, since Messages sends it) | `outbox send` to any number | `send_message` to any number |
| Open at login, installing updates on start | — | Settings → Open at login, off until turned on (LaunchAgent, Windows Run key, XDG autostart); a login launch installs a waiting update straight away | — | — |
| Photos, files and group chats | Yes: group threads with each sender, photos opened in the browser, files to save | Yes, and sends photos and files through Messages | `inbox chats`, `inbox list --chat`, `inbox save` | `list_chats`, `save_attachment`, `search_messages` chat filter |
| Tapbacks and threaded replies | Yes (shown) | Yes (shown) | In `inbox list` | In message results |
| Reply in a group chat | Yes: queued to the group's Messages chat | Yes | `outbox send <send_to>` | `send_message` to `send_to` |
| Send later | 🕘 beside Send | Later beside Send | `outbox send --at +2h` | `send_message` `send_at`/`delay_minutes` |
| Names and photos from the computer's address book | — (browsers can't read it) | macOS: Settings → Mac Contacts sends a sealed card (`"copy":"device"`, stored as the phone copy) for each sender it matches; Windows and Linux have no address book API to read yet | Gap: reads cards, can't read Mac Contacts | Gap: reads cards only |
| Switch between papamo.dev and this computer's archive | Hosted only | Settings → Where your texts live, with a confirmation that explains the switch, plus Sync to web (Plus) | `manage cloud connect` | — |

## BlueBubbles parity

pSMS aims to be BlueBubbles with end-to-end encryption, self-hosting and an iPhone Shortcut route for people without a Mac. BlueBubbles' server (Apache-2.0) reads `~/Library/Messages/chat.db` and sends through Messages with AppleScript; a separate Private API helper needs System Integrity Protection turned off. pSMS for Mac does the first part the same way (`desktop-ui/src/mac_messages.rs`, Settings → Messages on this Mac, Full Disk Access) and stores everything sealed: the server sees opaque chat IDs and envelopes only.

| BlueBubbles feature | pSMS | Notes |
| --- | --- | --- |
| Receive iMessage and SMS, both directions | Yes (Everything mode) | Polls new chat.db rows; Photos & groups mode keeps the Shortcut for plain texts and replaces its copy when both arrive |
| Text stored in `attributedBody` | Yes | Same NSString/typedstream approach as BlueBubbles' parser |
| Attachments | Yes | Sealed per message (`/messages/{id}/attachments/{n}`), HEIC converted to JPEG, 1.5 MB per file, 100 MB per papamo.dev account |
| Group chats and members | Yes | `/chats` holds sealed title, members and the Messages chat identifier |
| Tapbacks (receive) | Yes | Sealed `/messages/{id}/reactions/{id}`; removing one deletes it; they don't count as texts |
| Threaded replies (receive) | Yes | `reply_to` inside the sealed record |
| Send text, send attachments, send to groups | Yes on the Mac; text to groups from every surface | AppleScript `chat id` and `POSIX file`, as BlueBubbles does without its helper. Gap: sending a file from web, CLI or MCP needs an upload path to the Mac |
| Scheduled messages | Yes | Outbox `send_at` on both servers |
| Contacts | Yes | Phone cards, Mac Contacts and edits |
| Webhooks, REST API, push | Yes | Webhooks, `/messages`, SSE and papamo.dev events |
| Search, statistics | Search yes; statistics gap | — |
| Edits and unsends (receive) | Gap | BlueBubbles re-polls by date; pSMS reads new rows only. Needs a date-based pass that seals an edit record |
| Delivered/read receipts for sent texts | Gap | Same reason as edits |
| Group renames, participant changes as events | Gap | Rows with `item_type` 1–3 are skipped today |
| Send tapbacks, typing indicators, edit, unsend, mark read on the Mac, rename groups, add or remove people, effects, mentions, Focus status | Not planned for now | All need BlueBubbles' Private API, which needs SIP disabled. That weakens a core macOS protection, which conflicts with pSMS's privacy promise; revisit only as an explicit, warned opt-in |
| Find My, FaceTime | Not planned | Out of scope for a message archive |

Run `cargo test --offline -- --ignored copies_messages_into_a_running_archive` in `desktop-ui` with `PSMS_MESSAGES_TEST="<api url> <owner token>"` against a running local service to exercise the whole copy path with a synthetic chat.db.

## Product scope

Each recipient has a separate encrypted archive. Bundled SQLite is the default. Settings and first-run onboarding also support PostgreSQL, MySQL, MariaDB and SQL Server. The original SQLite file stays available for backup after migration. Verify the active destination with `psms storage status`.

The desktop groups incoming messages into conversations and supports dark mode. On a Mac it replies through the Messages app with AppleScript, using Text Message Forwarding for SMS when the iPhone allows it, and saves each sent reply sealed in the archive through `POST /v1/messages`.

### Plans

Hosted accounts have a plan (`accounts.plan` → `plans`): Free, Plus and Enterprise. Self-hosted pSMS has no plans or limits.

- Free: the latest 100 messages (a new message deletes the oldest unstarred one; an all-starred full archive refuses with 507), 3 phones, replies, autofill, contact cards, and 100 API calls per 30 minutes. Plus and Enterprise are unlimited and include webhooks; both are not yet open for sign-up (no billing yet), and the pricing page offers a waitlist or contact link.
- `archive_quota` gives each account's effective limits and gates; admins (`admin_role_snapshot`) are unlimited. Gates: `max_phones` on `/pair`, `replies` on `/computers` and `/outbox`, `autofill` on `/auth/browser`, all answering 402. `webhooks` is reported for hosted webhooks when they are built.
- API throttle (`throttle` in `hosted/src/main.rs`): counts requests made with computer tokens per account in 30-minute windows, with `x-ratelimit-*` headers and 429 plus `Retry-After`. The signed-in inbox, phone uploads and the Mac relay's claim and result calls are not counted.
- `GET /free/v1/plans` is public and cached (five minutes in memory and `Cache-Control: public, max-age=300`); papamo.dev/sms renders it with five-minute revalidation. Admins edit plans in the inbox (Account → Edit plans), through `GET/PUT /free/v1/admin/plans[/{id}]`; saving clears the server cache and revalidates /sms. The runtime role can update only the editable plan columns.
- Admins move accounts between plans by hand in the same dialog (search by name or account number), through `GET /free/v1/admin/accounts?q=` and `PUT /free/v1/admin/accounts/{id}/plan`. There is no billing yet, so this is how paid plans are granted.

### Plus for self-hosters and teams

Plus (plan id `plus`, 5,000 messages) and Enterprise (everything in Plus, capacity set per account by admins, plus support) unlock:

- Connect a self-hosted archive (`src/cloud.rs`, hosted `src/sync.rs`). The local service links with a papamo.dev computer token (`PUT /v1/cloud`), then pushes messages not yet synced (`POST /free/v1/sync/push`, original times kept) and pulls new ones with a microsecond cursor (`GET /free/v1/sync/pull`). Phones can therefore send to papamo.dev with no tunnel; pulled messages fire local webhooks through the usual triggers. Contact cards sync both ways by timestamp; computer tokens write the edited copy. `POST /v1/cloud/restore` resets the cursor to pull everything onto a new computer. Stars and read status do not sync yet. External SQL storage is not supported.
- Team inboxes (hosted `src/teams.rs`): owners add members by account number; members send `x-psms-inbox: <owner id>` (the site proxy adds it from the inbox picked in Settings) and unlock with the owner's recovery words. Saved keys are kept per inbox in the browser.
- Managed webhooks (hosted `src/webhooks.rs`): up to 10 HTTPS destinations; a database trigger queues `message.created` deliveries for plans with webhooks; every region's worker claims due deliveries with `SKIP LOCKED`, pins DNS to a public address (private, loopback, link-local, CGNAT and metadata addresses are refused), sends `{delivery_id, event, message_id}` with the destination's secret as a bearer token, and retries with backoff for 10 attempts. `GET /webhooks/deliveries` is the log; `POST …/{id}/replay` retries.
- Reply from anywhere already works for any account: queued texts wait on papamo.dev until the Mac is back, and pSMS opens at login.

### papamo.dev inbox on the desktop

Desktop setup no longer makes its own hosted shortcut or key. "Use my papamo.dev inbox" checks `GET /phones`; with no phone it opens `papamo.dev/sms/inbox#connect`, which starts the web QR pairing with the account's key. It then gets a computer token and asks for the recovery words; `Relay::check` verifies them against the inbox and `relay::save` keeps the token and key in the Keychain. Whenever the relay points at papamo.dev, the desktop shows that inbox instead of the local archive, unlocked with the Keychain key (`desktop-ui/src/relay.rs` `page`, `star`, `mark_read`, `save_card`, `store_sent`). It refetches a page only when `newest` from the relay's outbox claim grows or the page or filter changes, so viewing barely uses the free plan's API calls.

### One-time codes on the desktop

`desktop-ui/src/otp.rs` finds 4–8 digit codes in texts that mention a code (or the `@site #code` autofill format) and skips amounts, times and phone numbers. New local messages are checked while the archive is unlocked; the relay checks papamo.dev or remote inboxes using `newest` from `POST /outbox/claim`, fetching `GET /messages?limit=5` only when it grows. On macOS the app (as pSMS.app) posts a UserNotifications banner with **Type code** and **Copy code**; development builds without a bundle fall back to an osascript banner. With Accessibility permission, a CGEvent tap swallows a plain Tab and types the code while it is under a minute old. Codes live only in memory. Settings → One-time codes turns this off and asks for Accessibility.

### Releasing

GitHub Actions builds nothing on pushes to main; check locally first (AGENTS.md). To release, raise the version in `Cargo.toml` and `desktop-ui/Cargo.toml`, commit, and run from a clean `main`:

- **On the Mac:** `scripts/release.sh`. It runs every check and test, builds macOS with the disk image and, with Docker running (`colima start --vm-type vz --vz-rosetta`), both Linux architectures through `scripts/build-linux.sh` (Ubuntu 24.04 image in `packaging/linux/build.Dockerfile`, same steps as CI). Actions then builds only Windows.
- **On Windows:** `scripts/release.ps1`. It builds Windows with the NSIS installer (and Linux, given Docker and Git Bash). Actions then builds only macOS.

Both upload `local-papamo-sms-<target>.tar.gz` bundles to a draft release and start `build.yml` with `tag` and `skip`. Its `plan` job drops the skipped targets, the `release` job unpacks the bundles next to the Actions artifacts, signs everything with the key that exists only in Actions, publishes, and `rollout` updates the servers. A manual run of `build.yml` with a tag and no `skip` builds everything on Actions. Server-only changes still go out with `scripts/deploy-local.sh`.

### macOS installer

`packaging/macos/build-dmg.sh` builds `psms-macos-arm64.dmg` with pSMS.app (launcher `Contents/MacOS/launch` runs `psms desktop`; the CLI is inside as `psms`). It is ad-hoc signed, not notarized, so the first open needs right-click → Open or System Settings → Open Anyway. Name nothing in `Contents/MacOS` `pSMS`: the file system is case-insensitive and it would replace `psms`. CI builds it on the macOS runner and the release attaches it; the tar.gz stays available.

### Reply from anywhere

The goal: sign in from any computer, reply, and the Mac at home sends it through Messages while it is on. It works the same for papamo.dev accounts and self-hosted servers behind Cloudflare Tunnel, ngrok or similar.

- Web inbox: seals `{to, body}` with the archive key and queues it (`outbox-send` → hosted `POST /free/v1/outbox {envelope}`). The server never sees the recipient or text. Pending replies show under the conversation and update live; the sent copy arrives through the normal push feed.
- pSMS for Mac (`desktop-ui/src/relay.rs`): Settings → Reply from anywhere. For papamo.dev it signs in and gets a year-long `computer` session from `POST /free/v1/computers`; for self-hosted it takes the API URL and an owner or API token. The token and archive key live in the login Keychain (`dev.papamo.psms.relay`), so it keeps working after restarts. A background thread claims the outbox every 5 seconds, opens envelopes (or reads `to`/`body` from a self-hosted queue), sends through Messages, saves the sealed sent copy with `POST /messages`, marks it read and reports the result. Computers are listed and removed with phones on the web.
- The Mac must be on with pSMS open. Setting up replies turns on Open at login (a LaunchAgent, `dev.papamo.psms`, running `psms desktop`), so texts queued while the Mac was off or asleep send once it is back. Each claim records the computer's `last_used_at`; `GET /outbox` returns `computer_seen_at`, and the inbox explains a waiting reply with when the Mac was last online. Texts wait in the outbox meanwhile; a claim returns to the queue after five minutes without a result.
- Devices-only API (`src/device_api.rs`): `psms manage devices enable` or Settings → Your devices opens a second listener (default port 47840) that serves only `POST /v1/outbox` and `GET /v1/outbox/{id}`, accepts only `send`-scope keys and refuses peers outside private ranges (LAN, link-local, Tailscale 100.64.0.0/10, IPv6 ULA). It is plain HTTP, so use it on trusted networks or Tailscale.
- Gaps: the CLI and MCP queue texts on the local archive only; they cannot yet target a hosted account. The relay needs macOS for Messages; an Android sender can use the same claim and result calls.

### Outbox

Any surface can queue a text with `POST /v1/outbox {to, body}` (owner token, or an app token with the `send` scope from `psms manage access create send`). A sending device claims work with `POST /v1/outbox/claim`, sends it, and reports `POST /v1/outbox/{id}/result {status: sent|failed, error?}`. Claims return to the queue after five minutes without a result. `GET /v1/outbox/{id}` shows the status. The service never sends anything itself and drops the text once a device reports it sent. pSMS for Mac is the sending device today and checks the queue every few seconds while it is open; an Android app can use the same three calls. Texts queued while the archive is locked are sent but not kept in the archive, because the desktop needs the key to seal the copy.

The desktop makes every change through the local API (`desktop-ui/src/api.rs`): stars, read status, contact edits, replies and the outbox. It finds the service from the `.ready` sidecar and the owner token in `local_credentials`, so it needs the archive service running to make changes; reading still opens the SQLite file. Archives on an external database still star through `psms storage desktop-star`. Settings manages API tokens, browser tokens, event webhooks and regional sync timing. Chrome and Firefox offer click-to-fill for recent site-bound codes. Install the extensions from the release ZIPs as described in the guide. Android capture is planned.

For an AI client, run `psms init` and register `psms mcp` as a stdio server. The tools support combined message filters, message lookup by UUID, contact expansion and paginated contact history. A recovery key enables local decryption and shares matching message content with the selected AI client.

The public product page, downloads and setup guide are available at [Papamo Software](https://www.papamo.dev/sms). Singapore, South America, North America and Warsaw run separate archives. Regional replication is asynchronous and checks available capacity. The default interval is six hours.
