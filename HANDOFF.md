# pSMS handoff

## Start and verify

1. Put the matching `psms` and `psms-desktop` executables in one directory. Run `psms init`. On Windows, find **pSMS** in the notification area, including its overflow menu.
2. Run `psms status` to see the selected local HTTP address. Right-click the tray icon and choose **Open** for the native GPUI Kit dashboard. **Settings** opens the inbox preferences. Close the window to leave ingestion running. **Exit** shuts down the daemon. On other platforms use `psms desktop`, `psms settings`, and `psms stop`.
3. Choose a reachable HTTPS origin. Forward an existing HTTPS reverse proxy to the reported local address, start a named Cloudflare tunnel, or run `psms tunnel quick` in a second terminal with cloudflared installed. A temporary URL changes when the connector restarts. For another provider point its outbound connector at the same HTTP origin. Verify that it passes the Authorization header and POST body. CGNAT permits this outbound setup.
4. Run `psms onboard --endpoint https://YOUR-HOST/v1/messages --output PRIVATE-NEW-FOLDER`. Keep the recovery key on a trusted device. Sign the private shortcut on macOS and import it on the phone. This step still requires Apple signing before distribution. For a second phone or an OpenPGP shortcut, match the encrypted wire format in README. The native viewer decrypts an Actions message with that message's archive key.
5. On an unlocked test phone, receive “pSMS test one”. Refresh the dashboard, enter the recovery key and select **Decrypt Actions message**. Repeat while locked, checking for a second row before unlocking. Mac-to-iPhone library sync was observed by the owner. Background delivery, signed distribution and actual iPGMail output need device testing.
6. Verify the open/close/exit menu experience manually on the Windows desktop. Verify Linux and both Mac architectures from CI artifacts. No public infrastructure or cloud account is provisioned by this checkout. The background process runs for the current session; boot-time service installation is future work.

## Step-by-step screenshots to collect

Use a dedicated test contact and messages. Capture just the app and record the exact iOS version, language, encryption app version and device. Hide keys, tokens, private shortcut links, real phone numbers, contact photos and notifications. Capture each step as a separate image.

| Filename suffix | iOS 27 flow | Earlier supported iOS flow to validate |
| --- | --- | --- |
| 01-provider | Encryption app installed on the receiving phone | Same |
| 02-add | Preview of signed pSMS shortcut, **Add Shortcut** visible | Preview of an earlier-version-compatible upload shortcut |
| 03-library | pSMS in Library after import or Mac sync | Uploaded shortcut in Library |
| 04-trigger | Open the shortcut editor, show its **Message** trigger | **Automation → New Automation → Message** |
| 05-filter | Show the space/phrase or selected test sender | Select the same test criterion |
| 06-enable | Show the enabled automation on the phone | Select **Run Immediately** and connect the upload shortcut |
| 07-permission | First network permission prompt for the test host | Same, if displayed |
| 08-first-upload | Desktop dashboard after the unlocked test | Same |
| 09-locked-upload | Desktop dashboard with a second timestamp while phone remains locked | Same, if supported |
| 10-decrypt | Local decryption of synthetic test content | Same |

Keep originals by version, e.g. `ios-27.1/04-trigger.png`. For releases with different screens, capture the actual sequence and note the difference. Docs home in the companion repository is `content/docs/papamo-sms.mdx`.

## Product scope

This build is single-owner SQLite with encrypted ingest, owner CRUD for archive metadata, a native inbox, and independent tunnel connectors. OpenPGP integrations supply their own tested shortcut. Contacts, related-resource expansion, webhook delivery, managed proxying, subscriptions and hosted tenant isolation remain backlog items. Raw archive text is never needed by the tunnel or daemon for new uploads.
