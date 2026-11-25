"""Exercise two real archives over loopback, including an interrupted synchronization."""
from contextlib import closing
import base64
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from uuid import uuid4

binary = str(Path(sys.argv[1]).resolve())
def cli(db, *args):
    subprocess.run([binary, "--database", str(db), *map(str,args)], check=True, stdout=subprocess.DEVNULL)

def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

def call(url, token, data, method="POST"):
    req = Request(url, method=method, data=json.dumps(data).encode(),headers={
        "Authorization":"Bearer "+token, "Content-Type":"application/json"})
    try:
        with urlopen(req,timeout=5) as res:
            return res.status
    except HTTPError as exc:
        exc.close()
        return exc.code

def wait(check):
    for _ in range(120):
        if check(): return
        time.sleep(0.25)
    raise AssertionError("timed out waiting for replication")

with tempfile.TemporaryDirectory() as folder:
    root=Path(folder)
    hub, source=root/"hub.db", root/"source.db"
    hp,sp=port(),port()
    hurl,surl=f"http://127.0.0.1:{hp}",f"http://127.0.0.1:{sp}"
    pairing=root/"sa.pairing.json"
    cli(hub,"sync","pair","--source","sa","--public-url",hurl,"--output",pairing)
    cli(source,"sync","connect",pairing)
    token,owner=secrets.token_hex(32),secrets.token_hex(32)
    env=dict(os.environ,PAPAMO_TOKEN=token,PAPAMO_OWNER_TOKEN=owner,PAPAMO_SYNC_INTERVAL_SECONDS="1")
    procs=[]
    def start(db,p):
        proc=subprocess.Popen([binary,"--database",str(db),"--bind",f"127.0.0.1:{p}","serve"],env=env,stderr=subprocess.PIPE,text=True)
        procs.append(proc)
        assert "Listening on" in proc.stderr.readline()
        return proc
    def stop(proc):
        proc.terminate()
        proc.wait(timeout=10)
        proc.stderr.close()
    def mirror(mid):
        with closing(sqlite3.connect(hub)) as db:
            return db.execute("SELECT revision,body,starred FROM replicated_messages WHERE region='sa' AND id=?",(mid,)).fetchone()
    try:
        sender=start(source,sp)
        mid=str(uuid4())
        msg=dict(id=mid,sender="papamo:encrypted:v1",body=base64.b64encode(secrets.token_bytes(50)).decode())
        assert call(surl+"/v1/messages",token,msg)==201
        assert call(surl+"/v1/messages",token,msg)==200
        time.sleep(2)
        stop(sender)
        receiver=start(hub,hp)
        sender=start(source,sp)
        wait(lambda: mirror(mid) and mirror(mid)[1]==msg["body"])
        rev=mirror(mid)[0]
        assert call(surl+"/v1/messages/"+mid,owner,{"starred":True},"PATCH")==204
        wait(lambda: mirror(mid)[2]==1)
        assert call(surl+"/v1/messages/"+mid,owner,None,"DELETE")==204
        wait(lambda: mirror(mid)[1] is None)
        snapshot=dict(revision=rev,id=mid,snapshot=dict(sender=msg["sender"],body=msg["body"],archived_at=0,starred=False))
        target=hurl+"/internal/sync/sa"
        assert call(target,"incorrect",[snapshot])==401
        assert call(target,json.loads(pairing.read_text())["token"],[snapshot])==200
        assert mirror(mid)[1] is None
        assert call(surl+"/v1/messages",token,msg)==410
        print("PASS: offline upload, restart, replication, retry, star, deletion and stale replay")
    finally:
        for proc in procs:
            if proc.poll() is None: stop(proc)

