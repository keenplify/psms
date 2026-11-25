"""Prepare a private, no-question encrypted shortcut before signing it on a Mac."""
import argparse
import os
import plistlib
import secrets
import string
from pathlib import Path
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="new private output directory")
    parser.add_argument("--endpoint", default="https://sms.papamo.dev/v1/messages")
    args = parser.parse_args()
    endpoint = urlsplit(args.endpoint)
    if endpoint.scheme != "https" or not endpoint.hostname or endpoint.username is not None or endpoint.query or endpoint.fragment:
        parser.error("provide an HTTPS messages URL without credentials, query, or fragment")
    token = os.environ.get("PAPAMO_TOKEN", secrets.token_hex(32))
    alphabet = string.ascii_letters + string.digits
    key = os.environ.get("PAPAMO_ARCHIVE_KEY") or "".join(secrets.choice(alphabet) for _ in range(32))
    if len(token) < 32 or not all(33 <= ord(c) <= 126 for c in token):
        parser.error("PAPAMO_TOKEN requires at least 32 visible ASCII characters")
    if len(key) != 32 or any(c not in alphabet for c in key) or key == token:
        parser.error("use a separate 32-character ASCII alphanumeric archive key")
    template = Path(__file__).with_name("Papamo SMS Encrypted.shortcut")
    workflow = plistlib.loads(template.read_bytes())
    for action, value in zip(workflow["WFWorkflowActions"][:3], (args.endpoint, token, key), strict=True):
        if action["WFWorkflowActionIdentifier"] != "is.workflow.actions.gettext":
            raise ValueError("unexpected configuration action in template")
        action["WFWorkflowActionParameters"]["WFTextActionText"] = value
    workflow["WFWorkflowImportQuestions"] = []
    args.directory.mkdir(mode=0o700, exist_ok=False)
    outputs = {
        "Papamo_SMS_Encrypted_Private.shortcut": plistlib.dumps(workflow, fmt=plistlib.FMT_BINARY, sort_keys=False),
        "upload-token.txt": (token + "\n").encode(),
        "archive-recovery-key.txt": (key + "\n").encode(),
    }
    for name, data in outputs.items():
        path = args.directory / name
        with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_BINARY", 0), 0o600), "wb") as dest:
            dest.write(data)


if __name__ == "__main__":
    main()
