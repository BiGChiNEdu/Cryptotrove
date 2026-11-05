// CryptoTrove agent. Runs on GitHub Actions every few minutes.
// 1) reads each vault from the NEAR contract
// 2) when its stage changes, asks NEAR AI Cloud (private inference) to write a short message
// 3) emails the owner (warning / cancelled) or the next of kin (released)
// 4) saves a public status note in agent/status.json (no emails, no secrets)
import fs from "node:fs";
import nodemailer from "nodemailer";

const CONTRACT = "cryptotrove.testnet";
const SITE = "https://bigchinedu.github.io/Cryptotrove/";
const RPCS = ["https://rpc.testnet.near.org", "https://test.rpc.fastnear.com"];
const AI_URL = "https://cloud-api.near.ai/v1/chat/completions";
const AI_MODEL = process.env.AI_MODEL || "zai-org/GLM-5.1-FP8";
const STATUS_FILE = "agent/status.json";

const contacts = JSON.parse(process.env.VAULT_CONTACTS || "[]");
const status = fs.existsSync(STATUS_FILE) ? JSON.parse(fs.readFileSync(STATUS_FILE, "utf8")) : {};

async function view(method, args) {
  const body = JSON.stringify({
    jsonrpc: "2.0", id: "1", method: "query",
    params: { request_type: "call_function", finality: "final", account_id: CONTRACT, method_name: method, args_base64: Buffer.from(JSON.stringify(args)).toString("base64") },
  });
  let last;
  for (const rpc of RPCS) {
    try {
      const j = await (await fetch(rpc, { method: "POST", headers: { "content-type": "application/json" }, body })).json();
      if (j.result && j.result.result) return JSON.parse(Buffer.from(j.result.result).toString());
      last = new Error(JSON.stringify(j.error || j));
    } catch (e) { last = e; }
  }
  throw last;
}

const SYSTEM = "You are the CryptoTrove vault agent. You write short, calm, plain-language messages about a digital legacy vault. You never mention or invent passphrases, seed phrases, passwords or private keys, because you do not have any. Only use the facts you are given. Maximum 80 words. No markdown, no greeting placeholders.";

async function ai(facts, fallback) {
  if (!process.env.NEAR_AI_API_KEY) return fallback;
  try {
    const r = await fetch(AI_URL, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: "Bearer " + process.env.NEAR_AI_API_KEY },
      body: JSON.stringify({ model: AI_MODEL, max_tokens: 400, messages: [{ role: "system", content: SYSTEM }, { role: "user", content: facts }] }),
    });
    const j = await r.json();
    const t = (j.choices?.[0]?.message?.content || "").replace(/<think>[\s\S]*?<\/think>/g, "").trim();
    if (t) return t;
    console.log("AI returned nothing, using fallback:", JSON.stringify(j).slice(0, 300));
  } catch (e) { console.log("AI error, using fallback:", e.message); }
  return fallback;
}

const mailer = process.env.GMAIL_ADDRESS
  ? nodemailer.createTransport({ service: "gmail", auth: { user: process.env.GMAIL_ADDRESS, pass: process.env.GMAIL_APP_PASSWORD } })
  : null;
async function mail(to, subject, text) {
  if (!mailer || !to) return console.log("EMAIL SKIPPED (no mail setup or address):", subject);
  await mailer.sendMail({ from: `CryptoTrove Agent <${process.env.GMAIL_ADDRESS}>`, to, subject, text });
  console.log("Email sent:", subject);
}

for (const c of contacts) {
  try {
    const v = await view("get_vault", { owner: c.owner });
    const now = v ? v.status : "None";
    const prev = status[c.owner]?.status || "New";
    if (now === prev) { console.log(c.owner, "unchanged:", now); continue; }
    console.log(c.owner, prev, "->", now);
    const releaseAt = v ? new Date(v.release_at_secs * 1000).toUTCString() : "";
    let message, fallback;

    if (now === "Grace") {
      fallback = `${c.owner} missed a check-in. The vault will be released to next of kin on ${releaseAt} unless the owner checks in first.`;
      message = await ai(`Vault owner ${c.owner} missed their check-in. The vault is in a grace period and will be released to their next of kin on ${releaseAt} unless they check in. Write a firm but kind warning to the owner.`, fallback);
      await mail(c.ownerEmail, "CryptoTrove: you missed a check-in", `${message}\n\nCheck in now to cancel the transfer: ${SITE}`);
    } else if (now === "Active" && prev === "Grace") {
      fallback = `${c.owner} checked in. The transfer was cancelled and the countdown restarted.`;
      message = await ai(`Vault owner ${c.owner} checked in during the grace period. The transfer to next of kin is cancelled and the check-in countdown has restarted. Write a short confirmation.`, fallback);
      await mail(c.ownerEmail, "CryptoTrove: transfer cancelled", message);
    } else if (now === "Released") {
      fallback = `The grace period for ${c.owner}'s vault has ended and the vault is now released to you.`;
      message = await ai(`The owner of vault ${c.owner} did not check in and the grace period has ended, so the vault is released to their next of kin. Write a gentle, clear message to the next of kin explaining that they can now open it with the passphrase the owner gave them in person.`, fallback);
      await mail(c.kinEmail, "CryptoTrove: a vault has been released to you",
        `${message}\n\nOpen the vault here: ${SITE}\nOwner account: ${c.owner}\nYou will need the passphrase the owner gave you in person. We do not have it and will never email it.`);
    } else if (now === "None") {
      message = fallback = `No vault found for ${c.owner}.`;
    } else {
      fallback = `${c.owner}'s vault is active and the owner has checked in on time.`;
      message = await ai(`Vault owner ${c.owner} has an active vault and is checking in on time. Write one short reassuring status line.`, fallback);
    }
    status[c.owner] = { status: now, message, updatedAt: new Date().toISOString() };
  } catch (e) { console.log("Error for", c.owner, e.message); }
}

fs.mkdirSync("agent", { recursive: true });
fs.writeFileSync(STATUS_FILE, JSON.stringify(status, null, 2));
