from pathlib import Path
import hashlib,sys,subprocess,sqlite3,json,uuid,tempfile
with tempfile.TemporaryDirectory() as root:
 db=Path(root)/"test.sqlite";cli=[sys.argv[1],"--database",str(db)]
 subprocess.run(cli+["--bind","127.0.0.1:0","init"],check=True,stdout=subprocess.DEVNULL)
 try:
  key="a"*32;kf=Path(root)/"private";kf.write_text(key)
  body=subprocess.check_output(["node","-e","const c=require('crypto'),iv=c.randomBytes(12),s=c.createCipheriv('aes-256-gcm',Buffer.from('a'.repeat(32)),iv);process.stdout.write(Buffer.concat([iv,s.update(JSON.stringify({sender:'Mika',body:'Saturday at noon'})),s.final(),s.getAuthTag()]).toString('base64'))"],text=True)
  mid=str(uuid.uuid4())
  with sqlite3.connect(db) as d:d.execute("INSERT INTO messages(id,sender,body) VALUES(?,'papamo:encrypted:v1',?)",(mid,body))
  cid=hashlib.sha256(f"psms-contact-v1\nMika\n{key}".encode()).hexdigest()
  with d:
   d.execute("INSERT INTO contacts(id) VALUES(?)",(cid,))
   d.execute("INSERT INTO message_contacts VALUES(?,?)",(mid,cid))
  d.close()
  p=subprocess.Popen(cli+["mcp","--key-file",str(kf)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
  def call(msg):
   p.stdin.write(json.dumps(dict(jsonrpc="2.0",**msg))+"\n");p.stdin.flush()
   if "id" in msg:return json.loads(p.stdout.readline())["result"]
  res=call(dict(id=1,method="initialize",params={"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"integration","version":"1"}}));assert "tools" in res["capabilities"]
  call(dict(method="notifications/initialized"))
  tools=call(dict(id=2,method="tools/list"))["tools"];assert {t["name"] for t in tools}=={"search_messages","get_message","list_contacts","contact_history"}
  def tool(id,name,arguments):
   res=call(dict(id=id,method="tools/call",params={"name":name,"arguments":arguments}))
   return json.loads(res["content"][0]["text"])
  assert tool(3,"search_messages",{"sender":"mika","text":"NOON","starred":False})["messages"][0]["id"]==mid
  assert tool(4,"search_messages",{"text":"wrong"})["messages"]==[]
  assert tool(5,"get_message",{"id":mid})["text"]=="Saturday at noon"
  assert tool(6,"contact_history",{"sender":"Mika"})["messages"][0]["text"]=="Saturday at noon"
  p.stdin.close();p.wait(10);assert p.returncode==0
  print("PASS stdio MCP: negotiated, listed four tools, searched by local sender/text/star and read message by UUID")
 finally:subprocess.run(cli+["stop"],check=True,stdout=subprocess.DEVNULL)
