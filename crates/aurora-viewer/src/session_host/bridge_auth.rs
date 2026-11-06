//! Purpose-bound mutual authentication for private loopback bridge lanes.
//! Only framed authentication payloads enter here: sockets, timeouts, frame
//! sizes, data directions and fresh session/lease checks belong to each lane.
//! A transport must cap authentication frames at MAX_AUTH_BYTES before reading
//! their bodies, even when its authenticated image frames have a larger cap.

use aurora_session::{MAX_WORKERS, WorkerIdentity};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const MAX_AUTH_BYTES: usize = 4096;
pub(crate) const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(1);
const VERSION: u16 = 1;
const CHAT_DOMAIN: &[u8] = b"aurora-session-chat/auth/v1";
const CONTROL_DOMAIN: &[u8] = b"aurora-session-preview-control/auth/v1";
const IMAGE_DOMAIN: &[u8] = b"aurora-session-preview-image/auth/v1";
const TRANSCRIPT_BYTES: usize = CONTROL_DOMAIN.len() + 1 + 32 + 16 + 16 + 8 + 32 + 32;

/// Chosen by the endpoint, never negotiated or accepted from an incoming DTO.
/// The chat domain and all three JSON authentication messages retain V1 bytes.
#[derive(Clone, Copy)]
pub(crate) enum Purpose {
    Chat,
    // Preview sockets consume these variants in the following checkpoint.
    #[allow(dead_code)]
    PreviewControl,
    #[allow(dead_code)]
    PreviewImage,
}

impl Purpose {
    fn domain(self) -> &'static [u8] {
        match self {
            Self::Chat => CHAT_DOMAIN,
            Self::PreviewControl => CONTROL_DOMAIN,
            Self::PreviewImage => IMAGE_DOMAIN,
        }
    }
}

/// Per-worker IPC key, serialized only inside the validated private bootstrap.
/// No public byte accessor or equality implementation exists; all proof
/// verification uses ring's constant-time HMAC verifier.
#[derive(Clone)]
pub(crate) struct BridgeSecret(Arc<[u8; 32]>);

impl BridgeSecret {
    pub fn random() -> io::Result<Self> {
        random_bytes()
            .and_then(Self::from_bytes)
            .map_err(|_| io::Error::other("session bridge key generation failed"))
    }

    fn from_bytes(bytes: [u8; 32]) -> Result<Self, AuthError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(AuthError::Rejected);
        }
        Ok(Self(Arc::new(bytes)))
    }
}

impl std::fmt::Debug for BridgeSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BridgeSecret([redacted])")
    }
}

impl Serialize for BridgeSecret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text: String = self.0.iter().map(|byte| format!("{byte:02x}")).collect();
        serializer.serialize_str(&text)
    }
}

impl<'de> Deserialize<'de> for BridgeSecret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 64 {
            return Err(serde::de::Error::custom("invalid private session bridge key"));
        }
        let mut bytes = [0; 32];
        for (pair, byte) in text.as_bytes().chunks_exact(2).zip(&mut bytes) {
            let digit = |digit| match digit {
                b'0'..=b'9' => Some(digit - b'0'),
                b'a'..=b'f' => Some(digit - b'a' + 10),
                _ => None,
            };
            let (Some(high), Some(low)) = (digit(pair[0]), digit(pair[1])) else {
                return Err(serde::de::Error::custom("invalid private session bridge key"));
            };
            *byte = high * 16 + low;
        }
        Self::from_bytes(bytes).map_err(|_| serde::de::Error::custom("invalid private session bridge key"))
    }
}

/// Supply only currently owned, live workers. Never format this capability or
/// authentication DTOs. Session/account/grid generations remain lane checks.
#[derive(Clone)]
pub(crate) struct Permit {
    pub identity: WorkerIdentity,
    pub lease_epoch: u64,
    pub secret: BridgeSecret,
}

impl Permit {
    pub fn valid(&self) -> bool {
        self.lease_epoch != 0
    }
}

/// Categories only: no authentication payload, key or parser excerpt escapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthError {
    Malformed,
    Rejected,
    Randomness,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "session bridge authentication frame is invalid",
            Self::Rejected => "session bridge authentication failed",
            Self::Randomness => "session bridge randomness is unavailable",
        })
    }
}

impl std::error::Error for AuthError {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    version: u16,
    nonce: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerProof {
    version: u16,
    identity: WorkerIdentity,
    lease_epoch: u64,
    nonce: [u8; 32],
    tag: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostProof {
    version: u16,
    tag: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
enum Authentication {
    Challenge(Challenge),
    Worker(WorkerProof),
    Accepted(HostProof),
}

#[derive(Clone, Copy)]
enum Role {
    Worker = 1,
    Host = 2,
}

struct Transcript {
    bytes: [u8; TRANSCRIPT_BYTES],
    length: usize,
}

/// Fixed endpoint domain, role, exact public boot identity/lease and both fresh
/// nonces. The key enters only ring::hmac, never the transcript or wire DTO.
fn transcript(purpose: Purpose, role: Role, permit: &Permit, server: &[u8; 32], client: &[u8; 32]) -> Transcript {
    let mut transcript = Transcript {
        bytes: [0; TRANSCRIPT_BYTES],
        length: 0,
    };
    let role = [role as u8];
    let lease = permit.lease_epoch.to_be_bytes();
    let parts: [&[u8]; 8] = [
        purpose.domain(),
        &role,
        permit.identity.installation.as_bytes(),
        permit.identity.host.as_bytes(),
        permit.identity.worker.as_bytes(),
        &lease,
        server,
        client,
    ];
    for part in parts {
        transcript.bytes[transcript.length..transcript.length + part.len()].copy_from_slice(part);
        transcript.length += part.len();
    }
    transcript
}

fn proof(purpose: Purpose, role: Role, permit: &Permit, server: &[u8; 32], client: &[u8; 32]) -> [u8; 32] {
    let transcript = transcript(purpose, role, permit, server, client);
    let key = hmac::Key::new(hmac::HMAC_SHA256, permit.secret.0.as_ref());
    let tag = hmac::sign(&key, &transcript.bytes[..transcript.length]);
    let mut output = [0; 32];
    // HMAC_SHA256 returns exactly 32 bytes, independent of external input.
    output.copy_from_slice(tag.as_ref());
    output
}

fn verifies(purpose: Purpose, role: Role, permit: &Permit, server: &[u8; 32], client: &[u8; 32], tag: &[u8; 32]) -> bool {
    let transcript = transcript(purpose, role, permit, server, client);
    let key = hmac::Key::new(hmac::HMAC_SHA256, permit.secret.0.as_ref());
    hmac::verify(&key, &transcript.bytes[..transcript.length], tag).is_ok()
}

fn encode(authentication: &Authentication) -> Result<Vec<u8>, AuthError> {
    let bytes = serde_json::to_vec(authentication).map_err(|_| AuthError::Malformed)?;
    if bytes.is_empty() || bytes.len() > MAX_AUTH_BYTES {
        Err(AuthError::Malformed)
    } else {
        Ok(bytes)
    }
}

fn decode(bytes: &[u8]) -> Result<Authentication, AuthError> {
    if bytes.is_empty() || bytes.len() > MAX_AUTH_BYTES {
        return Err(AuthError::Malformed);
    }
    serde_json::from_slice(bytes).map_err(|_| AuthError::Rejected)
}

fn random_bytes() -> Result<[u8; 32], AuthError> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| AuthError::Randomness)?;
    Ok(bytes)
}

/// One freshly accepted socket, consumed after its single worker proof. The
/// transport owns the one-second timeout and bounded unauthenticated framing.
pub(crate) struct ServerAuth {
    purpose: Purpose,
    server: [u8; 32],
}

impl ServerAuth {
    pub fn begin(purpose: Purpose) -> Result<(Self, Vec<u8>), AuthError> {
        let server = random_bytes()?;
        let bytes = encode(&Authentication::Challenge(Challenge {
            version: VERSION,
            nonce: server,
        }))?;
        Ok((Self { purpose, server }, bytes))
    }

    pub fn accept(self, bytes: &[u8], allowed: &[Permit]) -> Result<(Permit, Vec<u8>), AuthError> {
        if allowed.len() > MAX_WORKERS
            || allowed.iter().enumerate().any(|(index, permit)| {
                !permit.valid()
                    || allowed[..index].iter().any(|earlier| earlier.identity == permit.identity)
                    || allowed.first().is_some_and(|first| {
                        (first.identity.installation, first.identity.host) != (permit.identity.installation, permit.identity.host)
                    })
            })
        {
            return Err(AuthError::Rejected);
        }
        let worker = match decode(bytes)? {
            Authentication::Worker(worker) if worker.version == VERSION => worker,
            _ => return Err(AuthError::Rejected),
        };
        let mut permit = None;
        for candidate in allowed {
            // Check all bounded candidates with ring's maintained verifier.
            // No key-derived identifier is sent to select a candidate.
            let valid = verifies(self.purpose, Role::Worker, candidate, &self.server, &worker.nonce, &worker.tag);
            if valid && candidate.identity == worker.identity && candidate.lease_epoch == worker.lease_epoch {
                permit = Some(candidate.clone());
            }
        }
        let permit = permit.ok_or(AuthError::Rejected)?;
        let reply = encode(&Authentication::Accepted(HostProof {
            version: VERSION,
            tag: proof(self.purpose, Role::Host, &permit, &self.server, &worker.nonce),
        }))?;
        Ok((permit, reply))
    }
}

enum ClientPhase {
    Challenge,
    Accepted { server: [u8; 32], client: [u8; 32] },
    Finished,
}

/// An IO owner may enqueue data only after Ready. Error, replay or an out-of-
/// order message consumes the handshake; reconnect requires a new instance.
pub(crate) struct ClientAuth {
    purpose: Purpose,
    permit: Permit,
    phase: ClientPhase,
}

pub(crate) enum ClientStep {
    Reply(Vec<u8>),
    Ready,
}

impl ClientAuth {
    pub fn new(purpose: Purpose, permit: Permit) -> Result<Self, AuthError> {
        if !permit.valid() {
            return Err(AuthError::Rejected);
        }
        Ok(Self {
            purpose,
            permit,
            phase: ClientPhase::Challenge,
        })
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Result<ClientStep, AuthError> {
        let phase = std::mem::replace(&mut self.phase, ClientPhase::Finished);
        match (phase, decode(bytes)?) {
            (ClientPhase::Challenge, Authentication::Challenge(challenge)) if challenge.version == VERSION => {
                let client = random_bytes()?;
                let reply = encode(&Authentication::Worker(WorkerProof {
                    version: VERSION,
                    identity: self.permit.identity,
                    lease_epoch: self.permit.lease_epoch,
                    nonce: client,
                    tag: proof(self.purpose, Role::Worker, &self.permit, &challenge.nonce, &client),
                }))?;
                self.phase = ClientPhase::Accepted {
                    server: challenge.nonce,
                    client,
                };
                Ok(ClientStep::Reply(reply))
            }
            (ClientPhase::Accepted { server, client }, Authentication::Accepted(host))
                if host.version == VERSION && verifies(self.purpose, Role::Host, &self.permit, &server, &client, &host.tag) =>
            {
                Ok(ClientStep::Ready)
            }
            _ => Err(AuthError::Rejected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{HostId, InstallationId, WorkerId};

    const PURPOSES: [Purpose; 3] = [Purpose::Chat, Purpose::PreviewControl, Purpose::PreviewImage];

    fn permit() -> Permit {
        Permit {
            identity: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).expect("synthetic installation"),
                host: HostId::from_bytes([2; 16]).expect("synthetic host"),
                worker: WorkerId::from_bytes([3; 16]).expect("synthetic worker"),
            },
            lease_epoch: 7,
            secret: BridgeSecret::from_bytes([7; 32]).expect("synthetic private bootstrap key"),
        }
    }

    fn challenge(server: [u8; 32]) -> Vec<u8> {
        encode(&Authentication::Challenge(Challenge {
            version: VERSION,
            nonce: server,
        }))
        .expect("synthetic challenge")
    }

    fn worker(purpose: Purpose, permit: &Permit, server: &[u8; 32], client: [u8; 32]) -> Vec<u8> {
        encode(&Authentication::Worker(WorkerProof {
            version: VERSION,
            identity: permit.identity,
            lease_epoch: permit.lease_epoch,
            nonce: client,
            tag: proof(purpose, Role::Worker, permit, server, &client),
        }))
        .expect("synthetic worker proof")
    }

    fn host(purpose: Purpose, permit: &Permit, server: &[u8; 32], client: &[u8; 32]) -> Vec<u8> {
        encode(&Authentication::Accepted(HostProof {
            version: VERSION,
            tag: proof(purpose, Role::Host, permit, server, client),
        }))
        .expect("synthetic host proof")
    }

    fn waiting(purpose: Purpose, permit: Permit, server: [u8; 32], client: [u8; 32]) -> ClientAuth {
        ClientAuth {
            purpose,
            permit,
            phase: ClientPhase::Accepted { server, client },
        }
    }

    #[test]
    fn bridge_auth_chat_v1_transcript_and_json_remain_byte_compatible() {
        let permit = permit();
        let server = [8; 32];
        let client = [9; 32];
        for role in [Role::Worker, Role::Host] {
            // Independent frozen V1 layout: no purpose selector, padding or
            // length prefix may enter the existing chat MAC transcript.
            let mut legacy = b"aurora-session-chat/auth/v1".to_vec();
            legacy.push(role as u8);
            legacy.extend_from_slice(&[1; 32]);
            legacy.extend_from_slice(&[2; 16]);
            legacy.extend_from_slice(&[3; 16]);
            legacy.extend_from_slice(&7u64.to_be_bytes());
            legacy.extend_from_slice(&server);
            legacy.extend_from_slice(&client);
            let current = transcript(Purpose::Chat, role, &permit, &server, &client);
            assert_eq!(&current.bytes[..current.length], legacy);
            let legacy_key = hmac::Key::new(hmac::HMAC_SHA256, &[7; 32]);
            assert_eq!(
                proof(Purpose::Chat, role, &permit, &server, &client).as_slice(),
                hmac::sign(&legacy_key, &legacy).as_ref()
            );
        }

        let decimals = |bytes: &[u8]| bytes.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
        assert_eq!(
            String::from_utf8(challenge(server)).expect("challenge JSON"),
            format!(
                "{{\"type\":\"challenge\",\"data\":{{\"version\":1,\"nonce\":[{}]}}}}",
                decimals(&server)
            )
        );
        let tag = proof(Purpose::Chat, Role::Worker, &permit, &server, &client);
        let bytes = worker(Purpose::Chat, &permit, &server, client);
        assert_eq!(
            String::from_utf8(bytes).expect("worker proof JSON"),
            format!(
                "{{\"type\":\"worker\",\"data\":{{\"version\":1,\"identity\":{},\"lease_epoch\":7,\"nonce\":[{}],\"tag\":[{}]}}}}",
                serde_json::to_string(&permit.identity).expect("public identity"),
                decimals(&client),
                decimals(&tag)
            )
        );
        let tag = proof(Purpose::Chat, Role::Host, &permit, &server, &client);
        assert_eq!(
            String::from_utf8(host(Purpose::Chat, &permit, &server, &client)).expect("host proof JSON"),
            format!("{{\"type\":\"accepted\",\"data\":{{\"version\":1,\"tag\":[{}]}}}}", decimals(&tag))
        );
    }

    #[test]
    fn bridge_auth_hmac_sha256_matches_rfc4231_and_binds_every_public_field() {
        // RFC 4231 section 4.2, https://www.rfc-editor.org/rfc/rfc4231.html#section-4.2
        let vector = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b, 0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9,
            0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c, 0x2e, 0x32, 0xcf, 0xf7,
        ];
        let key = hmac::Key::new(hmac::HMAC_SHA256, &[0x0b; 20]);
        assert_eq!(hmac::sign(&key, b"Hi There").as_ref(), vector);
        assert!(hmac::verify(&key, b"Hi There", &vector).is_ok());

        let permit = permit();
        let server = [8; 32];
        let client = [9; 32];
        for purpose in PURPOSES {
            let tag = proof(purpose, Role::Host, &permit, &server, &client);
            assert!(verifies(purpose, Role::Host, &permit, &server, &client, &tag));
            assert!(!verifies(purpose, Role::Worker, &permit, &server, &client, &tag), "role reflection");
            assert!(
                !verifies(purpose, Role::Host, &permit, &[10; 32], &client, &tag),
                "old server challenge"
            );
            assert!(
                !verifies(purpose, Role::Host, &permit, &server, &[10; 32], &tag),
                "old client challenge"
            );
            let mut changed = permit.clone();
            changed.lease_epoch += 1;
            assert!(!verifies(purpose, Role::Host, &changed, &server, &client, &tag));
            changed = permit.clone();
            changed.identity.installation = InstallationId::from_bytes([10; 32]).expect("other installation");
            assert!(!verifies(purpose, Role::Host, &changed, &server, &client, &tag));
            changed = permit.clone();
            changed.identity.host = HostId::from_bytes([10; 16]).expect("other host");
            assert!(!verifies(purpose, Role::Host, &changed, &server, &client, &tag));
            changed = permit.clone();
            changed.identity.worker = WorkerId::from_bytes([10; 16]).expect("other worker");
            assert!(!verifies(purpose, Role::Host, &changed, &server, &client, &tag));
            changed = permit.clone();
            changed.secret = BridgeSecret::from_bytes([10; 32]).expect("other synthetic key");
            assert!(!verifies(purpose, Role::Host, &changed, &server, &client, &tag));
        }
    }

    #[test]
    fn bridge_auth_same_key_cross_purpose_proofs_and_role_reflection_are_rejected() {
        let permit = permit();
        let server = [8; 32];
        let client = [9; 32];
        for (index, signing) in PURPOSES.iter().copied().enumerate() {
            for (other, verifying) in PURPOSES.iter().copied().enumerate() {
                for role in [Role::Worker, Role::Host] {
                    let tag = proof(signing, role, &permit, &server, &client);
                    assert_eq!(verifies(verifying, role, &permit, &server, &client, &tag), index == other);
                }
                if index != other {
                    assert!(
                        ServerAuth {
                            purpose: verifying,
                            server
                        }
                        .accept(&worker(signing, &permit, &server, client), std::slice::from_ref(&permit))
                        .is_err()
                    );
                    assert!(
                        waiting(verifying, permit.clone(), server, client)
                            .receive(&host(signing, &permit, &server, &client))
                            .is_err()
                    );
                }
            }
        }
    }

    #[test]
    fn bridge_auth_mutual_state_machines_complete_once_and_only_after_host_proof() {
        for purpose in PURPOSES {
            let permit = permit();
            let (server, challenge) = ServerAuth::begin(purpose).expect("fresh authentication challenge");
            let mut client = ClientAuth::new(purpose, permit.clone()).expect("synthetic client");
            let ClientStep::Reply(worker) = client.receive(&challenge).expect("worker proof") else {
                panic!("a public challenge cannot authenticate a server");
            };
            let (accepted, host) = server
                .accept(&worker, std::slice::from_ref(&permit))
                .expect("owned worker authentication");
            assert!(accepted.identity == permit.identity);
            assert_eq!(accepted.lease_epoch, permit.lease_epoch);
            assert!(matches!(client.receive(&host), Ok(ClientStep::Ready)));
            assert!(
                matches!(client.receive(&host), Err(AuthError::Rejected)),
                "an accepted proof cannot be replayed"
            );
        }
    }

    #[test]
    fn bridge_auth_replays_fail_against_either_new_challenge_and_retired_rosters() {
        let permit = permit();
        let server = [8; 32];
        let client = [9; 32];
        for purpose in PURPOSES {
            let bytes = worker(purpose, &permit, &server, client);
            assert!(
                ServerAuth { purpose, server: [10; 32] }
                    .accept(&bytes, std::slice::from_ref(&permit))
                    .is_err()
            );
            let acknowledged = host(purpose, &permit, &server, &client);
            assert!(waiting(purpose, permit.clone(), server, [10; 32]).receive(&acknowledged).is_err());
            assert!(waiting(purpose, permit.clone(), [10; 32], client).receive(&acknowledged).is_err());
            assert!(ServerAuth { purpose, server }.accept(&bytes, &[]).is_err());
            let replacement = Permit {
                lease_epoch: permit.lease_epoch + 1,
                ..permit.clone()
            };
            assert!(ServerAuth { purpose, server }.accept(&bytes, &[replacement]).is_err());
        }
    }

    #[test]
    fn bridge_auth_fake_host_cannot_reflect_worker_proof_or_recover_a_failed_client() {
        for purpose in PURPOSES {
            let permit = permit();
            let server = [8; 32];
            let mut client = ClientAuth::new(purpose, permit.clone()).expect("synthetic client");
            let ClientStep::Reply(reply) = client.receive(&challenge(server)).expect("public challenge") else {
                panic!("a challenge may only request the worker proof");
            };
            let Authentication::Worker(worker) = decode(&reply).expect("worker proof only") else {
                panic!("no data or secret is sent before server verification");
            };
            let reflected = encode(&Authentication::Accepted(HostProof {
                version: VERSION,
                tag: worker.tag,
            }))
            .expect("reflected public tag");
            assert!(matches!(client.receive(&reflected), Err(AuthError::Rejected)));
            assert!(matches!(
                client.receive(&host(purpose, &permit, &server, &worker.nonce)),
                Err(AuthError::Rejected)
            ));
        }
    }

    #[test]
    fn bridge_auth_fixed_caps_versions_unknown_fields_and_message_order_fail_closed() {
        assert!(matches!(decode(&[]), Err(AuthError::Malformed)));
        assert!(matches!(decode(&vec![b' '; MAX_AUTH_BYTES + 1]), Err(AuthError::Malformed)));
        let mut maximum = challenge([8; 32]);
        maximum.resize(MAX_AUTH_BYTES, b' ');
        assert!(matches!(decode(&maximum), Ok(Authentication::Challenge(_))));
        let mut value: serde_json::Value = serde_json::from_slice(&maximum).expect("synthetic challenge");
        value["data"]["purpose"] = "preview_image".into();
        assert!(decode(&serde_json::to_vec(&value).expect("unknown field")).is_err());

        for purpose in PURPOSES {
            let permit = permit();
            let mut client = ClientAuth::new(purpose, permit.clone()).expect("client");
            let out_of_order = host(purpose, &permit, &[8; 32], &[9; 32]);
            assert!(client.receive(&out_of_order).is_err());
            assert!(client.receive(&challenge([8; 32])).is_err());
            let mut client = ClientAuth::new(purpose, permit.clone()).expect("client");
            let unsupported = encode(&Authentication::Challenge(Challenge {
                version: VERSION + 1,
                nonce: [8; 32],
            }))
            .expect("wrong version");
            assert!(client.receive(&unsupported).is_err());
            let mut value: serde_json::Value = serde_json::from_slice(&worker(purpose, &permit, &[8; 32], [9; 32])).expect("proof");
            value["data"]["version"] = 2.into();
            assert!(
                ServerAuth { purpose, server: [8; 32] }
                    .accept(&serde_json::to_vec(&value).expect("wrong version"), std::slice::from_ref(&permit))
                    .is_err()
            );
        }
    }

    #[test]
    fn bridge_auth_secret_serde_is_private_redacted_and_never_enters_authentication_dtos() {
        let permit = permit();
        let encoded = serde_json::to_string(&permit.secret).expect("private bootstrap encoding");
        assert_eq!(encoded, format!("\"{}\"", "07".repeat(32)));
        let restored: BridgeSecret = serde_json::from_str(&encoded).expect("private bootstrap decoding");
        assert_eq!(format!("{restored:?}"), "BridgeSecret([redacted])");
        let tag = proof(Purpose::Chat, Role::Worker, &permit, &[8; 32], &[9; 32]);
        let restored = Permit {
            secret: restored,
            ..permit.clone()
        };
        assert!(verifies(Purpose::Chat, Role::Worker, &restored, &[8; 32], &[9; 32], &tag));
        for purpose in PURPOSES {
            let bytes = worker(purpose, &permit, &[8; 32], [9; 32]);
            let text = String::from_utf8(bytes).expect("public proof JSON");
            assert!(!text.contains(&"07".repeat(32)) && !text.contains("secret") && !text.contains("purpose"));
        }
        for invalid in ["", "00", &"00".repeat(32), &"FF".repeat(32), &"gg".repeat(32)] {
            assert!(serde_json::from_str::<BridgeSecret>(&format!("\"{invalid}\"")).is_err());
        }
    }

    #[test]
    fn bridge_auth_invalid_or_unbounded_admission_rosters_are_rejected() {
        let permit = permit();
        let purpose = Purpose::Chat;
        let bytes = worker(purpose, &permit, &[8; 32], [9; 32]);
        let duplicates = [permit.clone(), permit.clone()];
        assert!(ServerAuth { purpose, server: [8; 32] }.accept(&bytes, &duplicates).is_err());
        assert!(
            ServerAuth { purpose, server: [8; 32] }
                .accept(&bytes, &vec![permit.clone(); MAX_WORKERS + 1])
                .is_err()
        );
        let invalid = Permit {
            lease_epoch: 0,
            ..permit.clone()
        };
        assert!(ClientAuth::new(purpose, invalid.clone()).is_err());
        assert!(ServerAuth { purpose, server: [8; 32] }.accept(&bytes, &[invalid]).is_err());
        let mut foreign = permit.clone();
        foreign.identity.host = HostId::from_bytes([10; 16]).expect("other host");
        assert!(ServerAuth { purpose, server: [8; 32] }.accept(&bytes, &[permit, foreign]).is_err());
    }
}
