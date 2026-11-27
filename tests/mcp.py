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
  seal=lambda obj:subprocess.check_output(["node","-e","const c=require('crypto'),iv=c.randomBytes(12),s=c.createCipheriv('aes-256-gcm',Buffer.from('a'.repeat(32)),iv);process.stdout.write(Buffer.concat([iv,s.update(process.argv[1]),s.final(),s.getAuthTag()]).toString('base64'))",json.dumps(obj)],text=True)
  with sqlite3.connect(db) as d:d.execute("INSERT INTO contact_cards(contact_id,phone_card,phone_updated_at) VALUES(?,?,unixepoch())",(cid,seal({"fn":"Mika Santos","tel":"+15550100\n+15550101","email":["mika@example.com"]})))
  d.close()
  manage=lambda *a:subprocess.check_output(cli+["manage",*a],text=True)
  listed=json.loads(manage("contact","list","--key-file",str(kf)))["contacts"]
  assert listed[0]["card"]["name"]=="Mika Santos" and listed[0]["source"]=="phone" and [x["value"] for x in listed[0]["card"]["phones"]]==["+15550100","+15550101"]
  edited=json.loads(manage("contact","set","Mika","--key-file",str(kf),"--name","Mika S.","--phone","home=+1 555 0100","--phone","Kylla's office=+63 992 037 1891"))
  assert edited["id"]==cid and edited["card"]["phones"][1]=={"label":"Kylla's office","value":"+63 992 037 1891"} and edited["card"]["emails"][0]["value"]=="mika@example.com"
  assert json.loads(manage("contact","show",cid,"--key-file",str(kf)))["source"]=="edited"
  unread=json.loads(manage("inbox","list","--unread"))["messages"];assert [m["id"] for m in unread]==[mid] and unread[0]["read"] is False
  p=subprocess.Popen(cli+["mcp","--key-file",str(kf)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
  def call(msg):
   p.stdin.write(json.dumps(dict(jsonrpc="2.0",**msg))+"\n");p.stdin.flush()
   if "id" in msg:return json.loads(p.stdout.readline())["result"]
  res=call(dict(id=1,method="initialize",params={"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"integration","version":"1"}}));assert "tools" in res["capabilities"]
  call(dict(method="notifications/initialized"))
  tools=call(dict(id=2,method="tools/list"))["tools"];assert {t["name"] for t in tools}=={"search_messages","get_message","list_contacts","contact_history","update_message","set_contact","reset_contact","send_message","outbox_status","cloud_status","list_chats","save_attachment","phone_health"}
  def tool(id,name,arguments):
   res=call(dict(id=id,method="tools/call",params={"name":name,"arguments":arguments}))
   return json.loads(res["content"][0]["text"])
  assert tool(3,"search_messages",{"sender":"mika","text":"NOON","starred":False})["messages"][0]["id"]==mid
  assert tool(4,"search_messages",{"text":"wrong"})["messages"]==[]
  assert tool(5,"get_message",{"id":mid})["text"]=="Saturday at noon"
  assert tool(6,"contact_history",{"sender":"Mika"})["messages"][0]["text"]=="Saturday at noon"
  assert tool(7,"get_message",{"id":mid})["contact"]["name"]=="Mika S."
  assert tool(8,"list_contacts",{})["contacts"][0]["card"]["phones"][0]=={"label":"home","value":"+1 555 0100"}
  assert [m["id"] for m in tool(9,"search_messages",{"unread":True})["messages"]]==[mid]
  manage("inbox","read",mid)
  assert tool(10,"search_messages",{"unread":True})["messages"]==[] and tool(11,"get_message",{"id":mid})["read"] is True
  manage("contact","reset","Mika","--key-file",str(kf))
  assert tool(12,"get_message",{"id":mid})["contact"]["name"]=="Mika Santos"
  # Writes go through the same API as the CLI.
  assert tool(13,"update_message",{"id":mid,"starred":True,"read":False})["updated"] is True
  assert tool(14,"get_message",{"id":mid})["starred"] is True and tool(15,"get_message",{"id":mid})["read"] is False
  assert tool(16,"set_contact",{"contact":"Mika","title":"Chef","phones":["work=+1 555 0199"]})["card"]["phones"]==[{"label":"work","value":"+1 555 0199"}]
  assert tool(17,"list_contacts",{})["contacts"][0]["card"]["title"]=="Chef"
  assert tool(18,"reset_contact",{"contact":cid})["reset"] is True
  assert tool(19,"get_message",{"id":mid})["contact"]["name"]=="Mika Santos"
  queued=tool(20,"send_message",{"to":"+15550100","body":"On my way"});assert queued["status"]=="queued"
  assert tool(21,"outbox_status",{"id":queued["id"]})["status"]=="queued"
  cliq=json.loads(manage("outbox","send","+15550101","See you"));assert json.loads(manage("outbox","status",cliq["id"]))["status"]=="queued"
  # Group chats, tapbacks and files from pSMS for Mac, sealed like messages.
  chat="c"*32;gid=str(uuid.uuid4())
  with sqlite3.connect(db) as d:
   d.execute("INSERT INTO chats(id,envelope) VALUES(?,?)",(chat,seal({"title":"Lunch crew","group":True,"members":["+15550142","+15550187"],"identifier":"iMessage;+;chat777"})))
   d.execute("INSERT INTO messages(id,sender,body) VALUES(?,'papamo:encrypted:v1',?)",(gid,seal({"sender":"+15550187","body":"Look at this","thread":chat,"attachments":[{"type":"image/png","name":"IMG_1.png"}]})))
   d.execute("INSERT INTO message_chats VALUES(?,?)",(gid,chat))
   d.execute("INSERT INTO message_reactions(id,message_id,envelope) VALUES(?,?,?)",(str(uuid.uuid4()),gid,seal({"kind":"love","from":"me"})))
   d.execute("INSERT INTO message_reactions(id,message_id,envelope) VALUES(?,?,?)",(str(uuid.uuid4()),gid,seal({"kind":"edit","from":"","text":"Look at this one","at":5})))
   d.execute("INSERT INTO message_attachments(message_id,idx,envelope) VALUES(?,0,?)",(gid,seal("photo bytes")))
  d.close()
  chats=tool(22,"list_chats",{})["chats"];assert chats[0]["title"]=="Lunch crew" and chats[0]["send_to"]=="iMessage;+;chat777" and chats[0]["id"]==chat
  found=tool(23,"search_messages",{"chat":chat})["messages"];assert [m["id"] for m in found]==[gid]
  assert found[0]["text"]=="Look at this one" and found[0]["edited"] is True and found[0]["reactions"]==[{"kind":"love","from":"me"}] and found[0]["attachments"][0]["name"]=="IMG_1.png" and found[0]["thread"]==chat
  saved=tool(24,"save_attachment",{"id":gid,"directory":root});assert Path(saved["saved"]).read_bytes()==b'"photo bytes"'
  assert json.loads(manage("inbox","chats","--key-file",str(kf)))["chats"][0]["members"]==["+15550142","+15550187"]
  health=tool(25,"phone_health",{});assert "summary" in health and health["phones"][0]["id"]=="phone"
  assert json.loads(manage("health"))["summary"]==health["summary"]
  out=Path(root)/"saved.bin";manage("inbox","save",gid,"--key-file",str(kf),"--out",str(out));assert out.read_bytes()==b'"photo bytes"'
  listed=json.loads(manage("inbox","list","--chat",chat,"--key-file",str(kf)))["messages"];assert listed[0]["body"]=="Look at this one" and listed[0]["reactions"][0]["kind"]=="love"
  # Devices-only send API: a send key from a private address queues texts, nothing else works.
  import socket,time,urllib.request,urllib.error
  sock=socket.socket();sock.bind(("127.0.0.1",0));port=sock.getsockname()[1];sock.close()
  st=json.loads(manage("devices","enable","--port",str(port)));assert st["enabled"] and st["url"].endswith(f":{port}/v1/outbox") and st["key"] is None
  key=manage("access","create","send").strip().splitlines()[-1].strip()
  def dev(path,token,data=None):
   req=urllib.request.Request(f"http://127.0.0.1:{port}/v1{path}",json.dumps(data).encode() if data else None,{"Content-Type":"application/json","Authorization":"Bearer "+token})
   try:
    with urllib.request.urlopen(req,timeout=5) as r:return r.status,json.loads(r.read() or "null")
   except urllib.error.HTTPError as e:return e.code,None
   except OSError:return 0,None
  for _ in range(40):
   if dev("/outbox","x"*64,{"to":"1","body":"x"})[0]:break
   time.sleep(0.25)
  od=sqlite3.connect(db);owner=od.execute("SELECT owner FROM local_credentials").fetchone()[0];od.close()
  assert dev("/outbox",owner,{"to":"+15550100","body":"x"})[0]==401
  s2,q=dev("/outbox",key,{"to":"+15550100","body":"From my laptop"});assert s2==202,s2
  assert dev("/outbox/"+q["id"],key)[1]["status"]=="queued"
  assert dev("/messages",key)[0]==404
  manage("devices","disable")
  for _ in range(40):
   if dev("/outbox",key,{"to":"1","body":"x"})[0]==0:break
   time.sleep(0.25)
  else:raise AssertionError("device API still listening")
  assert tool(22,"cloud_status",{})=={"connected":False}
  assert json.loads(manage("cloud","status"))=={"connected":False}
  p.stdin.close();p.wait(10);assert p.returncode==0
  print("PASS stdio MCP and CLI: tools, local search, contact cards with labelled phones, edits and reset, read status, MCP writes, the outbox and the devices-only send API")
 finally:subprocess.run(cli+["stop"],check=True,stdout=subprocess.DEVNULL)
