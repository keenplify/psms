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

Hosted accounts all use one shared, signed shortcut named **pSMS**: https://www.icloud.com/shortcuts/cdb587ebe76f48758596453b60a34b20. It holds no credentials; its single import question receives each person's private setup. In **Connect a phone**, a computer shows a QR code that opens `papamo.dev/sms/pair` on the iPhone, with the setup in the link fragment. On an iPhone, the dialog shows the same buttons directly. The phone page copies the setup and opens the iCloud link, and the user pastes the setup and taps **Add Shortcut**. Step 4 of the dialog confirms the first run and the first real text. The iPhone frames are in the Papamo site under `public/images/psms/pair-phone-*.png`.

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


## Product scope

Each recipient has a separate encrypted archive. Bundled SQLite is the default. Settings and first-run onboarding also support PostgreSQL, MySQL, MariaDB and SQL Server. The original SQLite file stays available for backup after migration. Verify the active destination with `psms storage status`.

The desktop groups incoming messages into conversations and supports dark mode. Replies are disabled. Settings manages API tokens, browser tokens, event webhooks and regional sync timing. Chrome and Firefox offer click-to-fill for recent site-bound codes. Install the extensions from the release ZIPs as described in the guide. Android capture is planned.

For an AI client, run `psms init` and register `psms mcp` as a stdio server. The tools support combined message filters, message lookup by UUID, contact expansion and paginated contact history. A recovery key enables local decryption and shares matching message content with the selected AI client.

The public product page, downloads and setup guide are available at [Papamo Software](https://www.papamo.dev/sms). Singapore, South America, North America and Warsaw run separate archives. Regional replication is asynchronous and checks available capacity. The default interval is six hours.
