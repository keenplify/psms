# Sync two pSMS archives

Use sync if you want a second computer to hold a readable copy of encrypted messages from your first one. Both computers need pSMS. Choose one as the receiver and keep receiving phones pointed at their original archive. Your phone continues saving locally even when the receiver is offline.

1. On the **receiver**, expose its pSMS service at an HTTPS address accessible from the sender (a reverse proxy or stable tunnel works). Allow requests up to 2 MiB on the `/internal/sync/` path. Choose a unique source name for each sending archive, such as `home`.
2. On the receiver, run the command below. Transfer the resulting file privately to the sender. It contains a credential that can write replica data for this source.

   ```sh
   psms sync pair --source home --public-url https://YOUR-RECEIVER --output home.pairing.json
   ```

3. On the **sender**, run `psms sync connect home.pairing.json`. On each computer, restart the running instance with `psms stop` followed by `psms init`. For a system service, restart that service instead. Use the same OS account and database path used by your running instance for all commands. Store pairing files in a private directory.
4. Run `psms sync status` on the sender to see its pending message count. The first pass includes previously saved encrypted messages. Existing plaintext bodies are excluded.
5. On the receiver, use the owner's bearer token with `GET /v1/replicas/home/messages` to read the replica. The usual limit and cursor pagination parameters apply. Stars and deletions originate on the source. The native inbox shows the local archive.

## When data moves

Every 12 hours, the sender checks whether the machines have capacity. Each must have at least 192 MiB available. On Linux the one-minute load average must be below 0.65 per available CPU. On Windows and macOS measured CPU use must be below 65%. A deferral or failed request leaves changes on disk and retries after five minutes. Each batch carries up to eight messages, with a one-second pause between batches. Empty queues send no network requests. Set `PAPAMO_SYNC_INTERVAL_SECONDS` on the sending service to change the schedule (43200 by default).

Each regional save and queue update commits together. Multiple edits to a queued message collapse into its latest state. The receiver records revisions so a late retry cannot undo a newer edit or restore a deleted message. Replication can lag beyond 12 hours under sustained load or outages. It copies deletions too. Keep independent backups for point-in-time recovery.

For several computers, repeat pairing with a different source name per archive. Restore the source database and its pairing configuration together. After rolling a source back to an old backup, pair a fresh source identity to avoid replaying older revision numbers into an existing replica. To revoke a source, remove its entry in the receiver's private `<database>.sync-peers` directory and restart the receiver.
