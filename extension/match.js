const OTP_WORDS = /\b(code|otp|passcode|pass code|pin|verification|verify|one[- ]?time|password|2fa|mfa|login|log in|sign[- ]?in|authenticat\w*|security|confirm\w*)\b/i;
// 4-8 digits (optionally split 3+3 or 4+4) that are not part of an amount, date, time, phone number or longer number.
const CODE_RUN = /(?<![\d.,:/$€£¥₱₹#+])(\d{3,4}[ -]\d{3,4}|\d{4,8})(?![\d%]|[.,:/]\d)/g;

function unboundCode(body) {
  const text = body.replace(/\s+/g, " ");
  if (!OTP_WORDS.test(text)) return null;
  const found = [];
  for (const m of text.matchAll(CODE_RUN)) {
    const digits = m[1].replace(/[ -]/g, "");
    if (digits.length < 4 || digits.length > 8) continue;
    found.push({digits, index: m.index, length: m[1].length});
  }
  if (!found.length) return null;
  // Prefer a number right after an OTP word ("code is 123456", "OTP: 1234") or right before one ("123456 is your code").
  const near = found.filter(({index, length}) => {
    const before = text.slice(Math.max(0, index - 24), index);
    const after = text.slice(index + length, index + length + 32);
    return OTP_WORDS.test(before) || /^\W{0,3}(is|as)?\s*(your|the)?\s*[\w-]*\s*(verification |security |login |one-time )?(code|otp|pin|passcode|password)\b/i.test(after);
  });
  const pick = new Set((near.length ? near : found).map(f => f.digits));
  return pick.size === 1 ? [...pick][0] : null;
}

function extractSuggestion(message, hostname, received, now = Date.now(), unbound = false) {
  if (!Number.isFinite(received) || received * 1000 > now || now - received * 1000 >= 300000) return null;
  if (typeof message?.body !== "string") return null;
  const sender = typeof message.sender === "string" && /^[A-Za-z0-9 +()._-]{1,32}$/.test(message.sender) ? message.sender : "Messages";
  const expires = (received + 300) * 1000;
  const last = message.body.trimEnd().split(/\r?\n/).pop();
  const tag = /^@([a-z\d.-]+) #([0-9]{4,8})$/i.exec(last);
  if (tag) {
    // A tagged SMS names its site. Never offer it anywhere else, even in unbound mode.
    if (tag[1].toLowerCase() !== hostname.toLowerCase()) return null;
    return {sender, code: tag[2], expires, verified: true};
  }
  if (!unbound) return null;
  const code = unboundCode(message.body);
  return code ? {sender, code, expires, verified: false} : null;
}

if (typeof module === "object" && module.exports) module.exports = {extractSuggestion, unboundCode};
