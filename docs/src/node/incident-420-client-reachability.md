# #420 — Bounded external client reachability record

**Capture window:** 2026-10-23T22:43:39Z–22:45:03Z  
**Vantage:** connected Android/Termux client, independent of operator server hosts  
**Source:** public testnet endpoints listed in [#420](https://github.com/rhovision-org/rholo/issues/420)  
**Methods:** six read-only GET attempts, `curl --connect-timeout 2 --max-time 4`, no state-changing request or historical state exploration

## Raw outcome summary (sanitized, UTC)

Each record reports `curl` exit 28 (connection timeout), HTTP status 0 (no received HTTP response), and about 2 seconds to failure. An HTTP 0 is **not** an endpoint's actual HTTP response code.

~~~jsonl
{"time_utc":"2026-10-23T22:43:39Z","target":"A public health","url":"https://testnet.rhobot.net/health","http_code":0,"curl_exit":28,"latency_ms":2002.067,"outcome":"connect_timeout"}
{"time_utc":"2026-10-23T22:43:41Z","target":"A public LFB","url":"https://testnet.rhobot.net/api/last-finalized-block","http_code":0,"curl_exit":28,"latency_ms":2002.302,"outcome":"connect_timeout"}
{"time_utc":"2026-10-23T22:43:53Z","target":"B direct LFB","url":"http://104.131.176.164:40403/api/last-finalized-block","http_code":0,"curl_exit":28,"latency_ms":2001.557,"outcome":"connect_timeout"}
{"time_utc":"2026-10-23T22:43:55Z","target":"C direct LFB","url":"http://104.131.176.164:41403/api/last-finalized-block","http_code":0,"curl_exit":28,"latency_ms":2001.854,"outcome":"connect_timeout"}
{"time_utc":"2026-10-23T22:43:57Z","target":"A direct LFB","url":"http://164.90.140.144:40403/api/last-finalized-block","http_code":0,"curl_exit":28,"latency_ms":2002.071,"outcome":"connect_timeout"}
{"time_utc":"2026-10-23T22:43:59Z","target":"D direct LFB","url":"http://164.90.140.144:41403/api/last-finalized-block","http_code":0,"curl_exit":28,"latency_ms":2002.591,"outcome":"connect_timeout"}
~~~

The control request `HEAD https://github.com` at approximately `2026-10-23T22:45:03Z` succeeded with HTTP 200 (TCP connect approximately 0.308 s; total 0.749 s), establishing that this client was able to reach *another destination*. It does not exclude destination-specific routing/firewall/TLS or operator network failures.

**Interpretation:** These observations establish only that the six endpoints were unreachable from this client within the stated deadlines. They do **not** prove every validator was stopped, OOM-killed, missing a finalized fringe, or in a conflicting finalized state. No alternative route or expensive POST was attempted after connection failure.

## Relationship to later operator-side evidence

This is an immutable *client-vantage snapshot*. It does not supersede the separate operator-side confirmation of **four B/C-host RNode OOM kills** earlier that day, documented in [#420](https://github.com/rhovision-org/rholo/issues/420#issuecomment-6376322140). Conversely, B/C host OOM evidence is not proof that every GET timeout from this later client had the same cause.

The [engineering disposition](incident-420-engineering-disposition.md) records the other issue gates. Never reinterpret reachability failure as finalized state corruption without independently checked block hash/state and merge acceptance.
