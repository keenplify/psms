"""Black-box service and background lifecycle checks for a disposable archive."""
import base64
import json
import os
import plistlib
import re
import secrets
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


class ArchiveTest(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        self.db = self.root / "test.sqlite3"
        self.env = {k: v for k,v in os.environ.items() if not k.startswith("PAPAMO_")}
        self.env.update(PAPAMO_DB_PATH=str(self.db), PAPAMO_BIND="127.0.0.1:0", PAPAMO_API_PREFIX="/test/v1")

    def cli(self, *args):
        return subprocess.check_output([binary, *args], env=self.env, text=True, timeout=20).strip()

    def request(self, method, path, token, data=None):
        req = Request(self.base + path, method=method,
                      data=None if data is None else json.dumps(data).encode(),
                      headers={"Content-Type":"application/json","Authorization":"Bearer "+token})
        try:
            with urlopen(req, timeout=5) as response:
                body=response.read()
                return response.status, json.loads(body) if body else None
        except HTTPError as error:
            error.close()
            return error.code, None

    def test_service(self):
        upload, owner = secrets.token_hex(32),secrets.token_hex(32)
        self.env.update(PAPAMO_TOKEN=upload,PAPAMO_OWNER_TOKEN=owner)
        with subprocess.Popen([binary,"serve"],env=self.env,stderr=subprocess.PIPE,text=True) as proc:
            try:
                line=proc.stderr.readline()
                self.assertRegex(line, r"Listening on 127\.0\.0\.1:\d+")
                port=re.search(r":(\d+)",line).group(1)
                self.base=f"http://127.0.0.1:{port}/test/v1"
                raw=dict(id=uuid.uuid4().hex,sender="example",body="test")
                msg=dict(raw,sender="papamo:encrypted:v1",body=base64.b64encode(secrets.token_bytes(48)).decode())
                path="/messages/"+msg["id"]
                self.assertEqual(self.request("POST","/messages",upload,raw)[0],422)
                self.assertEqual(self.request("POST","/messages","wrong",msg)[0],401)
                self.assertEqual(self.request("POST","/messages",upload,msg)[0],201)
                self.assertEqual(self.request("POST","/messages",upload,msg)[0],200)
                self.assertEqual(self.request("GET","/messages",upload)[0],401)
                self.assertEqual(self.request("PATCH",path,owner,{"starred":True})[0],204)
                self.assertTrue(self.request("GET",path,owner)[1]["starred"])
                self.assertEqual(self.request("GET","/messages?limit=1",owner)[1]["messages"][0]["body"],msg["body"])
                self.assertEqual(self.request("DELETE",path,owner)[0],204)
                self.assertEqual(self.request("GET",path,owner)[0],404)
                self.assertEqual(self.request("POST","/messages",upload,msg)[0],410)
            finally:
                proc.terminate()
                proc.wait(timeout=10)

    def test_daemon_and_onboarding(self):
        self.assertIn("onboard",self.cli("--help"))
        self.assertIn("--token-file",self.cli("tunnel","cloudflare","--help"))
        self.assertFalse(self.db.exists())
        self.cli("init")
        self.addCleanup(self.cli,"stop")
        before=self.cli("status")
        self.assertIn("running",before)
        self.cli("init")
        self.assertEqual(self.cli("status"),before)
        address=re.search(r"http://\S+",before).group().rstrip(".")
        with urlopen(address+"/healthz",timeout=5) as response:self.assertEqual(response.status,204)
        self.cli("onboard","--endpoint","https://example.org/test/v1/messages","--output",str(self.root/"phone"))
        with (self.root/"phone/pSMS_Encrypted.shortcut").open("rb") as f: w=plistlib.load(f)
        self.assertEqual(w["WFWorkflowImportQuestions"],[])
        self.assertTrue(w["WFWorkflowActions"][1]["WFWorkflowActionParameters"]["WFTextActionText"]==self.cli("token","upload"))
        owner=self.cli("token","owner")
        self.cli("stop")
        self.assertIn("stopped",self.cli("status"))
        self.cli("init")
        self.assertTrue(owner==self.cli("token","owner"))


if __name__=="__main__":
    binary=str(Path(sys.argv.pop(1)).resolve())
    unittest.main()
