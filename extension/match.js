function extractSuggestion(message, hostname, received, now = Date.now()) {
  if (!Number.isFinite(received) || received * 1000 > now || now - received * 1000 >= 300000) return null;
  if (typeof message?.body !== "string") return null;
  const last = message.body.trimEnd().split(/\r?\n/).pop();
  const match = /^@([a-z\d.-]+) #([0-9]{4,8})$/i.exec(last);
  if (!match || match[1].toLowerCase() !== hostname.toLowerCase()) return null;
  const sender = typeof message.sender === "string" && /^[A-Za-z0-9 +()._-]{1,32}$/.test(message.sender) ? message.sender : "Messages";
  return {sender,code:match[2],expires:(received+300)*1000};
}
