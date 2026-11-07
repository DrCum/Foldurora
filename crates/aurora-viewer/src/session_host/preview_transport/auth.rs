//! Role/direction-separated mutual challenge authentication, following the
//! existing private chat lane pattern with independent preview domains/keys.

use super::{Error, Scope, Side};
use aurora_session::preview_wire::{Direction, HEADER_BYTES, Header, MAX_METADATA_BYTES, TAG_BYTES, VERSION};
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};

// Authentication schema remains V2; only image data framing/schema advances.
const AUTH_VERSION: u16 = 2;
const AUTH_DOMAIN: &[u8] = b"aurora-session-preview/auth/v1";
const DATA_DOMAIN: &[u8] = b"aurora-session-preview/data/v1";
const AUTH_MILLIS: u64 = 1000;
const AUTH_MAX: usize = 96;

pub(super) use super::super::preview_bootstrap::PreviewSecret;

fn random() -> Result<[u8; 32], Error> {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).map_err(|_| Error::Authentication)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(Error::Authentication);
    }
    Ok(bytes)
}

#[derive(Clone)]
pub(crate) struct AuthBytes {
    bytes: [u8; AUTH_MAX],
    len: usize,
}
impl AuthBytes {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
impl std::fmt::Debug for AuthBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewAuthentication([redacted])")
    }
}
fn envelope(scope: Scope, stage: u8, value: &[u8]) -> AuthBytes {
    let mut bytes = [0; AUTH_MAX];
    bytes[..4].copy_from_slice(b"APAU");
    bytes[4..6].copy_from_slice(&AUTH_VERSION.to_be_bytes());
    bytes[6] = scope.lane as u8;
    bytes[7] = stage;
    bytes[8..16].copy_from_slice(&scope.connection.to_be_bytes());
    bytes[16..24].copy_from_slice(&scope.role_epoch.to_be_bytes());
    bytes[24..24 + value.len()].copy_from_slice(value);
    AuthBytes {
        bytes,
        len: 24 + value.len(),
    }
}
fn check(scope: Scope, stage: u8, bytes: &[u8], len: usize) -> Result<&[u8], Error> {
    if !scope.valid()
        || bytes.len() != 24 + len
        || &bytes[..4] != b"APAU"
        || bytes[4..6] != AUTH_VERSION.to_be_bytes()
        || bytes[6] != scope.lane as u8
        || bytes[7] != stage
        || bytes[8..16] != scope.connection.to_be_bytes()
        || bytes[16..24] != scope.role_epoch.to_be_bytes()
    {
        return Err(Error::Authentication);
    }
    Ok(&bytes[24..])
}

/// Every proof/derived key binds full installation/host/worker/session/account/
/// grid/generation/lease, the role, connection generation and both fresh nonces.
fn context(
    mut context: hmac::Context,
    domain: &[u8],
    version: u16,
    role: &[u8],
    scope: Scope,
    server: &[u8; 32],
    client: &[u8; 32],
) -> hmac::Context {
    let worker = scope.source.worker();
    let session = scope.source.session();
    for part in [
        domain,
        &version.to_be_bytes(),
        role,
        &[scope.lane as u8],
        &scope.connection.to_be_bytes(),
        &scope.role_epoch.to_be_bytes(),
        worker.installation.as_bytes(),
        worker.host.as_bytes(),
        worker.worker.as_bytes(),
        session.session.as_bytes(),
        session.account.as_bytes(),
        session.grid.as_bytes(),
        &session.generation.to_be_bytes(),
        &scope.source.lease_epoch().to_be_bytes(),
        server,
        client,
    ] {
        context.update(part);
    }
    context
}
fn proof(secret: &PreviewSecret, role: u8, scope: Scope, server: &[u8; 32], client: &[u8; 32]) -> [u8; TAG_BYTES] {
    let tag = context(secret.context(), AUTH_DOMAIN, AUTH_VERSION, &[role], scope, server, client).sign();
    let mut bytes = [0; TAG_BYTES];
    bytes.copy_from_slice(tag.as_ref());
    bytes
}
fn verifies(secret: &PreviewSecret, role: u8, scope: Scope, server: &[u8; 32], client: &[u8; 32], tag: &[u8]) -> bool {
    // The outer HMAC makes ring::hmac::verify available for constant-time
    // verification of the domain/role-separated incremental proof digest.
    let digest = context(secret.context(), AUTH_DOMAIN, AUTH_VERSION, &[role], scope, server, client).sign();
    secret.verifies(digest.as_ref(), tag)
}
fn signed_proof(secret: &PreviewSecret, role: u8, scope: Scope, server: &[u8; 32], client: &[u8; 32]) -> [u8; TAG_BYTES] {
    let tag = secret.sign(&proof(secret, role, scope, server, client));
    let mut bytes = [0; TAG_BYTES];
    bytes.copy_from_slice(tag.as_ref());
    bytes
}
fn fresh(began: u64, now: u64) -> bool {
    now.checked_sub(began).is_some_and(|elapsed| elapsed < AUTH_MILLIS)
}

pub(crate) struct HostHandshake {
    scope: Scope,
    secret: PreviewSecret,
    server: [u8; 32],
    began: u64,
    consumed: bool,
}
impl HostHandshake {
    pub fn new(scope: Scope, secret: PreviewSecret, now: u64) -> Result<Self, Error> {
        if !scope.valid() {
            return Err(Error::Binding);
        }
        Ok(Self {
            scope,
            secret,
            server: random()?,
            began: now,
            consumed: false,
        })
    }
    pub fn challenge(&self) -> AuthBytes {
        envelope(self.scope, 1, &self.server)
    }
    pub fn accept(&mut self, bytes: &[u8], now: u64) -> Result<(AuthBytes, Authenticated), Error> {
        if self.consumed || !fresh(self.began, now) {
            self.consumed = true;
            return Err(Error::Expired);
        }
        self.consumed = true;
        let value = check(self.scope, 2, bytes, 64)?;
        let client: [u8; 32] = value[..32].try_into().map_err(|_| Error::Authentication)?;
        if client.iter().all(|byte| *byte == 0) || !verifies(&self.secret, 1, self.scope, &self.server, &client, &value[32..]) {
            return Err(Error::Authentication);
        }
        let accepted = envelope(self.scope, 3, &signed_proof(&self.secret, 2, self.scope, &self.server, &client));
        Ok((
            accepted,
            Authenticated::new(self.scope, Side::Host, &self.secret, &self.server, &client),
        ))
    }
}
pub(crate) struct WorkerHandshake {
    scope: Scope,
    secret: PreviewSecret,
    client: [u8; 32],
    server: Option<[u8; 32]>,
    began: u64,
    consumed: bool,
}
impl WorkerHandshake {
    pub fn new(scope: Scope, secret: PreviewSecret, now: u64) -> Result<Self, Error> {
        if !scope.valid() {
            return Err(Error::Binding);
        }
        Ok(Self {
            scope,
            secret,
            client: random()?,
            server: None,
            began: now,
            consumed: false,
        })
    }
    pub fn answer(&mut self, bytes: &[u8], now: u64) -> Result<AuthBytes, Error> {
        if self.consumed || self.server.is_some() || !fresh(self.began, now) {
            self.consumed = true;
            return Err(Error::Expired);
        }
        let server: [u8; 32] = match check(self.scope, 1, bytes, 32).and_then(|value| value.try_into().map_err(|_| Error::Authentication)) {
            Ok(server) => server,
            Err(error) => {
                self.consumed = true;
                return Err(error);
            }
        };
        if server.iter().all(|byte| *byte == 0) {
            self.consumed = true;
            return Err(Error::Authentication);
        }
        self.server = Some(server);
        let tag = signed_proof(&self.secret, 1, self.scope, &server, &self.client);
        let mut value = [0; 64];
        value[..32].copy_from_slice(&self.client);
        value[32..].copy_from_slice(&tag);
        Ok(envelope(self.scope, 2, &value))
    }
    pub fn finish(&mut self, bytes: &[u8], now: u64) -> Result<Authenticated, Error> {
        if self.consumed || !fresh(self.began, now) {
            self.consumed = true;
            return Err(Error::Expired);
        }
        self.consumed = true;
        let server = self.server.ok_or(Error::Authentication)?;
        let tag = check(self.scope, 3, bytes, 32)?;
        if !verifies(&self.secret, 2, self.scope, &server, &self.client, tag) {
            return Err(Error::Authentication);
        }
        Ok(Authenticated::new(self.scope, Side::Worker, &self.secret, &server, &self.client))
    }
}

pub(crate) struct Keys {
    header: hmac::Key,
    body: hmac::Key,
    finish: hmac::Key,
}
impl Keys {
    fn new(scope: Scope, direction: Direction, secret: &PreviewSecret, server: &[u8; 32], client: &[u8; 32]) -> Self {
        let derive = |purpose| {
            let tag = context(
                secret.context(),
                DATA_DOMAIN,
                VERSION,
                &[purpose, direction as u8],
                scope,
                server,
                client,
            )
            .sign();
            hmac::Key::new(hmac::HMAC_SHA256, tag.as_ref())
        };
        Self {
            header: derive(1),
            body: derive(2),
            finish: derive(3),
        }
    }
    pub fn header(&self, header: Header, metadata: &[u8]) -> [u8; TAG_BYTES] {
        let mut context = hmac::Context::with_key(&self.header);
        context.update(&header.encode());
        context.update(metadata);
        let tag = context.sign();
        let mut bytes = [0; TAG_BYTES];
        bytes.copy_from_slice(tag.as_ref());
        bytes
    }
    pub fn verify_header(&self, header: Header, metadata: &[u8], tag: &[u8]) -> Result<(), Error> {
        if metadata.len() != header.metadata_bytes() || metadata.len() > MAX_METADATA_BYTES || tag.len() != TAG_BYTES {
            return Err(Error::Invalid);
        }
        let mut input = [0; HEADER_BYTES + MAX_METADATA_BYTES];
        input[..HEADER_BYTES].copy_from_slice(&header.encode());
        input[HEADER_BYTES..HEADER_BYTES + metadata.len()].copy_from_slice(metadata);
        hmac::verify(&self.header, &input[..HEADER_BYTES + metadata.len()], tag).map_err(|_| Error::Authentication)
    }
    pub fn body(&self, header: Header, metadata: &[u8], tag: &[u8]) -> hmac::Context {
        let mut context = hmac::Context::with_key(&self.body);
        context.update(&header.encode());
        context.update(metadata);
        context.update(tag);
        context
    }
    pub fn finish(&self, body: hmac::Context) -> [u8; TAG_BYTES] {
        let tag = hmac::sign(&self.finish, body.sign().as_ref());
        let mut bytes = [0; TAG_BYTES];
        bytes.copy_from_slice(tag.as_ref());
        bytes
    }
    pub fn verify_finish(&self, body: hmac::Context, tag: &[u8]) -> Result<(), Error> {
        hmac::verify(&self.finish, body.sign().as_ref(), tag).map_err(|_| Error::Authentication)
    }
}
pub(crate) struct Authenticated {
    pub(super) scope: Scope,
    pub(super) side: Side,
    pub(super) send: Keys,
    pub(super) receive: Keys,
}
impl Authenticated {
    pub fn scope(&self) -> Scope {
        self.scope
    }
    fn new(scope: Scope, side: Side, secret: &PreviewSecret, server: &[u8; 32], client: &[u8; 32]) -> Self {
        Self {
            scope,
            side,
            send: Keys::new(scope, side.outgoing(), secret, server, client),
            receive: Keys::new(scope, side.incoming(), secret, server, client),
        }
    }
}
