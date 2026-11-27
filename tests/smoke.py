"""Black-box service and background lifecycle checks for a disposable archive."""
import base64
import hashlib
import json
import os
import plistlib
import re
import secrets
import sqlite3
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
        self.env = {k: v for k,v in os.environ.items() if not k.startswith(("PSMS_","PAPAMO_"))}
        self.env.update(PSMS_DB_PATH=str(self.db), PSMS_BIND="127.0.0.1:0", PSMS_API_PREFIX="/test/v1")

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

    def test_cli_settings_and_domain_proof(self):
        self.cli("init")
        self.addCleanup(self.cli,"stop")
        owner=self.cli("manage","access","create","api","--days","7")
        self.assertEqual(len(owner),64)
        self.assertNotEqual(self.cli("manage","access","rotate","api"),owner)
        self.cli("manage","access","revoke","api")
        self.assertEqual(self.cli("manage","access","status","api"),"No token")
        self.cli("manage","theme","dark")
        self.cli("manage","sync-interval","3600")
        hook=json.loads(self.cli("manage","webhook","add","https://example.org/psms-test"))
        self.assertNotIn(hook["token"],self.cli("manage","webhook","list"))
        self.cli("manage","webhook","remove",hook["id"])
        self.assertFalse(json.loads(self.cli("manage","inbox","list"))["messages"])
        with sqlite3.connect(self.db) as db:
            secret,prefix=db.execute("SELECT secret,prefix FROM tunnel_runtime").fetchone()
            self.assertEqual(prefix,"/test/v1")
            self.assertEqual(db.execute("SELECT seconds FROM sync_interval").fetchone()[0],3600)
        db.close()
        nonce=str(uuid.uuid4())
        origin="http://"+Path(str(self.db)+".ready").read_text().strip()
        with urlopen(origin+"/.well-known/psms/verify/"+nonce,timeout=5) as res:
            self.assertEqual(res.read().decode(),hashlib.sha256(f"psms-domain-v1/{secret}/{nonce}".encode()).hexdigest())
        result=subprocess.run([binary,"onboard","--domain","localhost","--output",str(self.root/"pair")],env=self.env,capture_output=True,timeout=8)
        self.assertNotEqual(result.returncode,0)
        self.assertFalse((self.root/"pair").exists())

    def test_service(self):
        upload, owner = secrets.token_hex(32),secrets.token_hex(32)
        self.env.update(PSMS_TOKEN=upload,PSMS_OWNER_TOKEN=owner)
        with subprocess.Popen([binary,"serve"],env=self.env,stderr=subprocess.PIPE,text=True) as proc:
            try:
                line=proc.stderr.readline()
                self.assertRegex(line, r"Listening on 127\.0\.0\.1:\d+")
                port=re.search(r":(\d+)",line).group(1)
                self.base=f"http://127.0.0.1:{port}/test/v1"
                raw=dict(id=uuid.uuid4().hex,sender="example",body="test")
                msg=dict(raw,sender="papamo:encrypted:v1",body=base64.b64encode(secrets.token_bytes(48)).decode())
                path="/messages/"+msg["id"]
                hooks = []
                for target in ("first","second"):
                    response, hook = self.request("POST","/webhooks",owner,{"url":f"https://127.0.0.1:1/{target}"})
                    self.assertEqual(response,201)
                    self.assertNotIn(hook["token"],(upload,owner))
                    hooks.append(hook)
                self.assertNotEqual(hooks[0]["token"],hooks[1]["token"])
                listing = self.request("GET","/webhooks",owner)[1]["webhooks"]
                self.assertTrue(all(set(h)=={"id","url"} for h in listing))
                for hook in hooks:
                    for method,resource,payload in (("GET","/contacts",None),("POST","/messages",msg),("POST","/webhooks/"+hooks[1]["id"]+"/rotate",None)):
                        self.assertEqual(self.request(method,resource,hook["token"],payload)[0],401)
                rotated = self.request("POST","/webhooks/"+hooks[0]["id"]+"/rotate",owner)[1]
                self.assertNotEqual(rotated["token"],hooks[0]["token"])
                self.assertEqual(self.request("POST","/messages",upload,raw)[0],422)
                self.assertEqual(self.request("POST","/messages","wrong",msg)[0],401)
                browser=self.cli("manage","access","create","browser","--days","7")
                for invalid in ("wrong",upload,owner):
                    self.assertEqual(self.request("GET","/otp/events",invalid)[0],401)
                req=Request(self.base+"/otp/events",headers={"Authorization":"Bearer "+browser})
                with urlopen(req,timeout=8) as stream:
                    def frame():
                        lines=[]
                        while True:
                            line=stream.readline()
                            if not line.strip(): return lines
                            lines.append(line.strip())
                    self.assertEqual(frame(),[b"event: ready",b"data: 1"])
                    temporary=dict(msg,id=uuid.uuid4().hex)
                    self.assertEqual(self.request("POST","/messages",upload,temporary)[0],201)
                    self.assertEqual(frame(),[b"event: arrival",b"data: 1"])
                    self.assertEqual(self.request("DELETE","/messages/"+temporary["id"],owner)[0],204)
                    self.cli("manage","access","revoke","browser")
                    self.assertEqual(self.request("GET","/otp/recent",browser)[0],401)
                self.assertEqual(self.request("POST","/messages",upload,msg)[0],201)
                self.assertEqual(self.request("POST","/messages",upload,msg)[0],200)
                contact = "a" * 64
                second = dict(msg, id=uuid.uuid4().hex, contact_id=contact.upper())
                self.assertEqual(self.request("POST","/messages",upload,second)[0],201)
                self.assertEqual(self.request("POST","/messages",upload,dict(second,contact_id=contact))[0],200)
                self.assertEqual(self.request("POST","/messages",upload,dict(second,contact_id="b"*64))[0],409)
                self.assertEqual(self.request("POST","/messages",upload,dict(second,id=uuid.uuid4().hex))[0],201)
                self.assertEqual(self.request("POST","/messages",upload,dict(second,contact_id="invalid"))[0],422)
                # Contact cards: the phone is asked for one until it sends a fresh copy.
                envelope = msg["body"]
                self.assertEqual(self.request("POST","/messages",upload,dict(second,id=uuid.uuid4().hex)),(201,{"contact_card":"wanted"}))
                card = "/contacts/"+contact+"/card"
                self.assertEqual(self.request("PUT",card,upload,{"envelope":envelope})[0],204)
                self.assertEqual(self.request("GET","/contacts/cards",upload)[0],401)
                cards = self.request("GET","/contacts/cards",owner)[1]["cards"]
                self.assertEqual([(c["contact_id"],c["phone_card"],c["manual_card"]) for c in cards],[(contact,envelope,None)])
                self.assertEqual(self.request("POST","/messages",upload,dict(second,id=uuid.uuid4().hex)),(201,{}))
                self.assertEqual(self.request("PUT",card,owner,{"envelope":envelope})[0],204)
                self.assertEqual(self.request("GET","/contacts/cards",owner)[1]["cards"][0]["manual_card"],envelope)
                self.assertEqual(self.request("PUT",card,owner,{"envelope":None})[0],204)
                self.assertIsNone(self.request("GET","/contacts/cards",owner)[1]["cards"][0]["manual_card"])
                self.assertEqual(self.request("PUT",card,upload,{"envelope":None})[0],422)
                self.assertEqual(self.request("PUT","/contacts/"+"A"*64+"/card",upload,{"envelope":envelope})[0],422)
                self.assertEqual(self.request("PUT",card,upload,{"envelope":"not base64"})[0],422)
                self.assertEqual(self.request("PUT",card,"wrong"*13,{"envelope":envelope})[0],401)
                with sqlite3.connect(self.db) as db:
                    self.assertEqual(db.execute("SELECT id FROM contacts").fetchall(),[(contact,)])
                    self.assertEqual(db.execute("SELECT count(*) FROM message_contacts").fetchone(),(4,))
                db.close()
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

    def test_two_recipients_same_sender(self):
        archives = []
        for index in range(2):
            db = self.root / f"inbox-{index}.db"
            owner, upload, key = (secrets.token_hex(32), secrets.token_hex(32), secrets.token_hex(16))
            proc = subprocess.Popen([binary, "--database", str(db), "serve"],
                                    env=dict(self.env,PSMS_OWNER_TOKEN=owner,PSMS_TOKEN=upload),
                                    stderr=subprocess.PIPE,text=True)
            self.addCleanup(proc.stderr.close)
            self.addCleanup(proc.wait,10)
            self.addCleanup(proc.terminate)
            port = re.search(r":(\d+)",proc.stderr.readline()).group(1)
            self.base = f"http://127.0.0.1:{port}/test/v1"
            key_file = self.root / f"key-{index}"
            key_file.write_text(key)
            cid = subprocess.check_output([binary,"contact-id","--key-file",str(key_file)],
                                          input="GLOBE\n",text=True).strip()
            self.assertEqual(cid,hashlib.sha256(f"psms-contact-v1\nGLOBE\n{key}".encode()).hexdigest())
            ids = []
            for _ in range(7):
                mid = str(uuid.uuid4())
                payload = dict(id=mid, sender="papamo:encrypted:v1",contact_id=cid,
                               body=base64.b64encode(secrets.token_bytes(48)).decode())
                self.assertEqual(self.request("POST","/messages",upload,payload)[0],201)
                ids.append(mid)
            archives.append((self.base,owner,upload,cid,ids))
        self.assertNotEqual(archives[0][3],archives[1][3])
        for i,(self.base,owner,upload,cid,ids) in enumerate(archives):
            _,other_owner,_,other_cid,other_ids = archives[1-i]
            for path in ("/contacts",f"/contacts/{cid}?include=messages",f"/contacts/{cid}/messages",
                         f"/messages/{ids[0]}","/replicas/sa/messages"):
                self.assertEqual(self.request("GET",path,other_owner)[0],401)
                self.assertEqual(self.request("GET",path,upload)[0],401)
            self.assertEqual(self.request("GET","/replicas/sa/messages",owner)[0],401)
            self.assertEqual(self.request("GET",f"/contacts/{other_cid}/messages",owner)[0],404)
            self.assertEqual(self.request("GET",f"/messages/{other_ids[0]}",owner)[0],404)
            self.assertEqual(self.request("GET",f"/contacts?contact_id={other_cid}",owner)[1]["contacts"],[])
            listing = self.request("GET",f"/contacts?contact_id={cid}&include=messages",owner)[1]["contacts"]
            self.assertEqual([c["id"] for c in listing],[cid])
            page = listing[0]["messages"]
            seen = [m["id"] for m in page["messages"]]
            self.assertEqual(len(seen),5)
            rest = self.request("GET",f"/contacts/{cid}/messages?cursor={page['next_cursor']}",owner)[1]
            self.assertCountEqual(seen+[m["id"] for m in rest["messages"]],ids)
            self.assertIsNone(rest["next_cursor"])

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
