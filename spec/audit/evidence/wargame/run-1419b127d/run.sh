export const meta = {
  name: 'wargame-pilot2',
  description: 'Wargame pilot v2: referee disarmed from the register, dedup fixed, proposer must name a line',
  phases: [
    { title: 'Reachability', detail: 'one agent per node: who reaches it, cited file:line' },
    { title: 'Propose', detail: 'persona actors sweep the eleven guide words' },
    { title: 'Referee', detail: 'bare-claim confirm/refute on each finding + the seeded controls' },
  ],
}

const GUIDE_WORDS = ['NO','MORE','LESS','AS WELL AS','PART OF','OTHER THAN','REVERSE','EARLY','LATE','BEFORE','AFTER']

const NODES = {
  N1:  { intent: "a peer's message is authenticated and bounded in size and rate", census: 'equivocation, withholding', personas: ['Eve','Mallory','Oscar','Alice'] },
  N2:  { intent: 'a deploy is admitted once; admitting is bounded per unit time per reachable identity', census: 'deploy flooding', personas: ['Eve','Mallory','Oscar','Bob'] },
  N3:  { intent: 'the public reads disclose only what a public reader is meant to see', census: 'new surface', personas: ['Eve','Bob'] },
  N4:  { intent: 'only the trusted operator can make the node act with its own authority', census: 'operator error', personas: ['Mallory','Operator'] },
  N5:  { intent: 'the signing key exists only where the operator intends and can be replaced', census: 'key compromise', personas: ['Root'] },
  N6:  { intent: 'a compromise of the node process is contained by the host', census: 'new surface', personas: ['Root'] },
  N7:  { intent: 'the artefact an operator runs is what the source builds, verifiably', census: 'new surface', personas: ['Root'] },
  N8:  { intent: 'the capability wire lets a stranger do only what a stranger is meant to do', census: 'new surface', personas: ['Mallory'] },
  N9:  { intent: 'the statement under test: a bounded validator can neither split safety nor stall liveness', census: 'all census rows', personas: ['Alice','Oscar'] },
  N10: { intent: 'an operator can see production, finality and halt state', census: 'new surface', personas: ['Operator'] },
  N11: { intent: 'the operator can advance and restore the node without a wipe', census: 'silence of a quorum; operator error', personas: ['Root','Operator'] },
  N12: { intent: 'no reachable action costs the node superlinearly in attacker input', census: 'deploy flooding', personas: ['Mallory'] },
  N13: { intent: 'the names a deploy can forge are unpredictable', census: 'adjacent to key compromise', personas: [] },
  N14: { intent: "the node's notion of time is bounded and observable", census: 'a validator rejecting its own block', personas: ['Oscar'] },
}

const PERSONA = {
  Eve: 'a PASSIVE network adversary: you can read every plaintext byte on the deploy gRPC port (40401) and the public HTTP port (40403) and observe timing. You CANNOT modify traffic.',
  Mallory: 'an ACTIVE network adversary: you can inject, modify, replay and drop traffic on any UNAUTHENTICATED surface, and you are a man in the middle.',
  Oscar: 'a BYZANTINE consensus participant: you are the statement\'s at-most-one malicious validator (under a third of the stake). You can sign two blocks at a height, withhold, selectively delay, and lie inside the protocol.',
  Root: 'a HOST or SUPPLY-CHAIN intruder: you can read the data directory, the key files, the process environment and argv, the container image base, and the release artefact. You are NOT the operator.',
  Operator: 'the TRUSTED operator: you restart, upgrade, reconcile, and choose recovery anchors. You are a FAULT source, not an adversary.',
  Alice: 'an HONEST validator operating a bonded node correctly (the non-adversary control).',
  Bob: 'an HONEST client, deployer and public reader (the control).',
}

const nid = s => { const m = String(s).match(/^N\d+/); return m ? m[0] : String(s) }
const sel = (args && args.nodes) ? args.nodes : Object.keys(NODES)
log(`wargame pilot2: nodes = ${sel.join(', ')}`)

const REACH_SCHEMA = {
  type: 'object',
  properties: {
    nodeId: { type: 'string' },
    reached: { type: 'array', items: { type: 'object', properties: {
      persona: { type: 'string' }, fact: { type: 'string' }, citation: { type: 'string' }
    }, required: ['persona','fact','citation'] } },
    blocked: { type: 'array', items: { type: 'object', properties: {
      persona: { type: 'string' }, why: { type: 'string' }
    }, required: ['persona','why'] } },
  },
  required: ['nodeId','reached','blocked'],
}

phase('Reachability')
const reach = (await parallel(sel.map(id => () => agent(
  `Repo: /home/patrick/RNodeRust (a Rust RChain node). Study node ${id}: "${NODES[id].intent}".

Read the code and decide, for each persona below, whether it can ACT against this function of a running node. For each that CAN, give the reachability fact and a file:line citation you re-derive yourself. For each that CANNOT, say why (a barrier). Do not consult spec/findings.tsv, spec/AUDIT.md, spec/audit/passes.md or docs/src/node/security-audit.md — work from the source alone.
Return nodeId as exactly "${id}".
Personas: ${NODES[id].personas.map(p => p + ' = ' + PERSONA[p]).join(' | ') || 'none seeded — judge which of ' + Object.keys(PERSONA).join('/') + ' can reach it'}.`,
  { schema: REACH_SCHEMA, label: `reach:${id}`, phase: 'Reachability' }
)))).filter(Boolean)
const reachByNode = {}
reach.forEach(r => { reachByNode[nid(r.nodeId || r.node)] = r })
log(`reachability: ${reach.map(r => nid(r.nodeId || r.node) + '=' + r.reached.length + 'r/' + r.blocked.length + 'b').join(' ')}`)

const PROPOSE_SCHEMA = {
  type: 'object',
  properties: {
    nodeId: { type: 'string' },
    persona: { type: 'string' },
    findings: { type: 'array', items: { type: 'object', properties: {
      word: { type: 'string' },
      deviation: { type: 'string' },
      class: { type: 'string' },
      inModel: { type: 'boolean' },
      reachFact: { type: 'string' },
      falsifier: { type: 'string' },
    }, required: ['word','deviation','class','inModel','reachFact','falsifier'] } },
    vacuous: { type: 'array', items: { type: 'object', properties: {
      word: { type: 'string' }, why: { type: 'string' }
    }, required: ['word','why'] } },
  },
  required: ['nodeId','persona','findings','vacuous'],
}

phase('Propose')
const props = []
for (const id of sel) {
  const facts = (reachByNode[id] && reachByNode[id].reached) || []
  for (const p of NODES[id].personas) props.push({ id, p, facts: facts.filter(f => f.persona === p) })
}
const swept = (await parallel(props.map(({ id, p, facts }) => () => agent(
  `Repo: /home/patrick/RNodeRust (a Rust RChain node). You ARE this persona, acting against study node ${id}: "${NODES[id].intent}".
PERSONA: ${p} — ${PERSONA[p]}
Your reachability facts (re-derive and extend): ${JSON.stringify(facts)}
Return nodeId exactly "${id}".

Run the eleven HAZOP guide words (${GUIDE_WORDS.join(', ')}) against this node, AS THIS PERSONA. Rules:
- A deviation is admissible ONLY if you can name the EXACT file:line where the deviating mechanism lives — put that citation in reachFact. A word with no such line is NOT a deviation.
- If two guide words describe the SAME mechanism, propose it once and list the second in "vacuous" as "folded into <word>".
- If there is NO bounded property the word could deviate from, put it in "vacuous" with the structural reason. Do not manufacture a deviation to fill a word — that is a defect, not a finding.
- For each real deviation give: class (unauth-actuation, authority-widening, key-at-rest, resource-amplification, disclosure, observability-gap, recovery-unbuilt, supply-chain-integrity, clock-entropy-substrate, plaintext-transit), whether it is inside the bounded-adversary model (false for host/supply-chain/operator-as-attacker), and a FALSIFIER (a test/measurement that would fire, or 'owed: <what is missing>').
Work from the source only; do NOT consult spec/findings.tsv, spec/AUDIT.md, spec/audit/passes.md or security-audit.md. Return raw JSON.`,
  { schema: PROPOSE_SCHEMA, label: `propose:${id}:${p}`, phase: 'Propose' }
)))).filter(Boolean)

const findings = []
const vacuous = []
swept.forEach(s => {
  const node = nid(s.nodeId || '')
  s.findings.forEach(f => findings.push({ node, persona: s.persona, ...f }))
  s.vacuous.forEach(v => vacuous.push({ node, word: v.word, why: v.why }))
})
const vacRate = (vacuous.length + findings.length) ? Math.round(100 * vacuous.length / (vacuous.length + findings.length)) : 0
log(`propose: ${findings.length} findings, ${vacuous.length} vacuous (vacuity ${vacRate}%)`)

// dedup by (nodeId, word) — cross-persona duplicates collapse, first kept
const seen = new Set()
const fresh = findings.filter(f => { const k = f.node + '|' + f.word; if (seen.has(k)) return false; seen.add(k); return true })
log(`dedup: ${findings.length} -> ${fresh.length}`)

const CONTROLS = [
  { node: 'N4', word: 'PART OF', deviation: "The admin server's origin guard is not an authentication check: it passes any request that carries no Origin header.", class: 'unauth-actuation', falsifier: 'a request with no Origin header reaching /api/propose', _expect: 'CONFIRMED', _kind: 'known-true' },
  { node: 'N4', word: 'MORE', deviation: 'The admin HTTP server binds to 0.0.0.0 (all interfaces) by default.', class: 'unauth-actuation', falsifier: 'the default admin bind host in configuration', _expect: 'REFUTED', _kind: 'false' },
  { node: 'N4', word: 'OTHER THAN', deviation: "An adversary who controls the operator's container image registry serves a backdoored rnode image that signs blocks with a key of their choosing.", class: 'supply-chain-integrity', falsifier: 'a block signed by an image the source did not build', _expect: 'NOT-RE-ESTABLISHED', _kind: 'out-of-model' },
  { node: 'N2', word: 'PART OF', deviation: 'The external deploy gRPC surface binds api-server.host with no authentication and no caller identity beyond the deploy signature.', class: 'plaintext-transit', falsifier: 'the deploy service bind in the node runtime', _expect: 'CONFIRMED', _kind: 'known-true' },
  { node: 'N3', word: 'NO', deviation: 'The public HTTP server requires a bearer token or API key on its read routes.', class: 'unauth-actuation', falsifier: 'an authentication middleware on the public router', _expect: 'REFUTED', _kind: 'false' },
  { node: 'N9', word: 'OTHER THAN', deviation: 'A quantum computer forges a validator signature and finalises a conflicting block.', class: 'clock-entropy-substrate', falsifier: 'a forged signature the others accept', _expect: 'NOT-RE-ESTABLISHED', _kind: 'out-of-model' },
]

const VERDICT_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { enum: ['CONFIRMED','NOT-RE-ESTABLISHED','REFUTED'] },
    artifact: { type: 'string' },
    reason: { type: 'string' },
  },
  required: ['verdict','artifact','reason'],
}

phase('Referee')
const queue = fresh.map(f => ({ claim: f, control: false })).concat(CONTROLS.map(c => ({ claim: c, control: true })))
const verdicts = (await parallel(queue.map(({ claim }) => () => agent(
  `You are an adversarial REFEREE. A candidate finding about the Rust RChain node at /home/patrick/RNodeRust is handed to you. Your ONLY job is to confirm or refute it by re-deriving the mechanism FROM THE CODE. You are given nothing else: not the reasoning, not who proposed it, not the other findings.
CANDIDATE FINDING
- node: ${claim.node}
- guide word: ${claim.word}
- deviation (one sentence): ${claim.deviation}
- class: ${claim.class}
- claimed falsifier: ${claim.falsifier}

RULES:
0. FIRST, in "reason", state in one phrase the EXACT mechanism the claim asserts (e.g. "the read routes check a bearer token" / "the faucet signs with the deployer key"). Then find the line where THAT mechanism runs.
1. Decide from the SOURCE alone. Do NOT read spec/findings.tsv, spec/AUDIT.md, spec/audit/passes.md or docs/src/node/security-audit.md — a register row is NOT evidence at this stage, and a row's existence (or absence) must not decide your verdict.
2. CONFIRMED — you can name the SPECIFIC expression, handler or branch where the claimed effect happens, and it does what the claim says. The artifact must be that specific site, not a bare file path and not a line that merely exists nearby (a router definition, a table header, a struct field). If all you can cite is a file, or a line that only shows a RELATED capability, the claim is NOT-RE-ESTABLISHED, not CONFIRMED. Do NOT retreat to NOT-RE-ESTABLISHED merely because no test exists — the code path is the artifact — but do not confirm on a line that does not itself perform the claimed mechanism.
3. REFUTED — you hold a POSITIVE ARTIFACT showing the mechanism is absent (a named line that contradicts it, a test, a measurement). Put it in "artifact". An empty-handed refutation is NOT permitted.
4. NOT-RE-ESTABLISHED — use when the claim's causal chain contains a step that is NOT in this tree (a compromised host, a compromised registry or mirror, a quantum adversary), or when you cannot point to the specific site where the claimed mechanism runs, or when the mechanism cannot be settled by reading (races, load-driven behaviour, crafted-depth exhaustion). You can produce no positive artifact either way.
Be sceptical but commit to the SPECIFIC SITE: if the line where the claimed effect happens exists and does what the claim says, that is CONFIRMED; anything less is NOT-RE-ESTABLISHED. Return raw JSON.`,
  { schema: VERDICT_SCHEMA, label: `referee:${claim.node}:${claim.word}`, phase: 'Referee' }
)))).filter(Boolean)

const rows = queue.map((q, i) => {
  const v = verdicts[i] || {}
  return { node: q.claim.node, word: q.claim.word, deviation: q.claim.deviation.slice(0, 80),
    control: q.control, kind: q.control ? q.claim._kind : '', expect: q.control ? q.claim._expect : '',
    verdict: v.verdict || '(dropped)', artifact: (v.artifact || '').slice(0, 110) }
})
const controls = rows.filter(r => r.control)
const caught = controls.filter(r => r.verdict === r.expect).length
const leaks = rows.filter(r => /findings\.tsv|AUDIT\.md|passes\.md|security-audit/.test(r.artifact)).length
log(`referee: ${rows.length} judged; calibration ${caught}/${controls.length}; register-leaks ${leaks}`)

return { nodes: sel, reachability: reach, nFindingsRaw: findings.length, findings: fresh, vacuousRate: vacRate, verdicts: rows, calibration: { caught, planted: controls.length, leaks } }
