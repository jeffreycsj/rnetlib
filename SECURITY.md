# Security scope

Unified endpoints created with `rnet_server_open_v2` and `rnet_client_connect_v2` always perform a `Noise_XX_25519_ChaChaPoly_BLAKE2s` handshake. The client verifies the server static public key through runtime-scoped trust policy. The server receives the client static public key and encrypted join payload and must explicitly accept the session.

The server-selected `PLAINTEXT` mode applies only to business records: the authenticated encrypted control channel remains active so a later upgrade, downgrade, or rekey cannot be forged. Plaintext business records provide neither confidentiality nor integrity. `ENCRYPTED` mode protects the complete logical frame. A downgrade is accepted only after an authenticated server-led barrier, but all business traffic after completion is intentionally visible.

TCP uses ordered Noise transport state. UDP and KCP use explicit 64-bit nonces and a 64-packet replay window, so valid limited reordering does not desynchronize decryption. Authentication-tag failures, pinned-key mismatches, and replays do not deliver application frames.

UDP/KCP listeners issue an HMAC cookie bound to source IP, source port, and a 60-second bucket before allocating Noise/session state. The current implementation accepts the current and previous bucket. This limits spoofed-source allocation and amplification but is not a substitute for network ingress rate limiting. Per-IP adaptive admission limits and distributed denial-of-service protection remain deployment responsibilities.

The compatibility API `rnet_endpoint_open` creates the original unauthenticated TCP/UDP endpoints. The older `rnet_listener_open`/`rnet_client_join` pair creates always-encrypted sessions. Treat only the low-level plaintext API as unsuitable for untrusted networks.

The cryptographic implementation is pinned to `snow 0.10.0`, `ring 0.17.14`, and `zeroize 1.9.0`. `snow` is not represented here as formally audited; production deployments with high-value secrets should arrange an independent review of the complete protocol integration and key-management system. RNet generates keys but does not provide a keystore, certificate authority, revocation service, or key rotation service.

KCP is pinned to `kcp 0.6.0`, a Rust translation of the reference algorithm. KCP provides reliability, not authentication or encryption. RNet uses one KCP conversation per peer socket address; applications needing multiple simultaneous logical connections from the same UDP 4-tuple require a future negotiated-conversation extension.

The C ABI cannot prove that an arbitrary non-null pointer is readable. Callers must keep input pointers valid for the documented call duration and must not release an event buffer while another thread reads it. Private-key copies owned by C/C++/Go callers must be cleared by those callers.

Please report vulnerabilities privately to the owning security team rather than opening a public issue with exploit details.
