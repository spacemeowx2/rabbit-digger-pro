use std::{
    io::{self, Cursor, ErrorKind, Read, Write},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aes_gcm::{AeadInPlace, Aes256Gcm, KeyInit};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use rd_interface::{error::map_other, AsyncRead, AsyncWrite, Result};
use reality::{RealityConnectionState, X25519RealityGroup};
use reality_rustls::crypto::ring::default_provider;
use reality_rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use reality_rustls::{
    client::Resumption, client::WebPkiServerVerifier, sign::CertifiedKey as RustlsCertifiedKey,
    sign::Signer as RustlsSigner, sign::SigningKey as RustlsSigningKey, sign::SingleCertAndKey,
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
    SignatureAlgorithm, SignatureScheme,
};
use sha2::{Sha256, Sha512};
use tls_parser::{
    parse_tls_client_hello_extensions, parse_tls_handshake_client_hello, SNIType, TlsExtension,
};
use tokio::io::ReadBuf;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

#[derive(Clone, Debug)]
pub(crate) struct RealityConfig {
    pub public_key: String,
    pub short_id: Option<String>,
    pub client_fingerprint: Option<String>,
}

impl RealityConfig {
    pub fn validate_client_fingerprint(&self) -> Result<()> {
        if let Some(fp) = self.client_fingerprint.as_deref() {
            if !fp.is_empty() && !fp.eq_ignore_ascii_case("chrome") {
                return Err(rd_interface::Error::other(format!(
                    "unsupported reality client fingerprint: {fp}"
                )));
            }
        }
        Ok(())
    }

    fn decode_public_key(&self) -> io::Result<[u8; 32]> {
        let mut public_key_bytes = [0u8; 32];
        if let Ok(b) = hex::decode(&self.public_key) {
            if b.len() == 32 {
                public_key_bytes.copy_from_slice(&b);
                return Ok(public_key_bytes);
            }
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(&self.public_key)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if decoded.len() != 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid reality public key length",
            ));
        }
        public_key_bytes.copy_from_slice(&decoded);
        Ok(public_key_bytes)
    }

    fn decode_short_id(&self) -> io::Result<[u8; 8]> {
        let mut short_id_bytes = [0u8; 8];
        if let Some(short_id) = self.short_id.as_deref() {
            if !short_id.is_empty() {
                let padded_short_id = format!("{:0<16}", short_id);
                hex::decode_to_slice(&padded_short_id[..16], &mut short_id_bytes).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid reality short id: {e}"),
                    )
                })?;
            }
        }
        Ok(short_id_bytes)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RealityServerConfig {
    pub server_name: String,
    pub private_key: String,
    pub short_id: Option<String>,
    pub max_time_diff: Option<Duration>,
}

impl RealityServerConfig {
    fn decode_private_key(&self) -> io::Result<[u8; 32]> {
        let mut private_key_bytes = [0u8; 32];
        if let Ok(b) = hex::decode(&self.private_key) {
            if b.len() == 32 {
                private_key_bytes.copy_from_slice(&b);
                return Ok(private_key_bytes);
            }
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(&self.private_key)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        if decoded.len() != 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid reality private key length",
            ));
        }
        private_key_bytes.copy_from_slice(&decoded);
        Ok(private_key_bytes)
    }

    fn decode_short_id(&self) -> io::Result<[u8; 8]> {
        let mut short_id_bytes = [0u8; 8];
        if let Some(short_id) = self.short_id.as_deref() {
            if !short_id.is_empty() {
                let padded_short_id = format!("{:0<16}", short_id);
                hex::decode_to_slice(&padded_short_id[..16], &mut short_id_bytes).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid reality short id: {e}"),
                    )
                })?;
            }
        }
        Ok(short_id_bytes)
    }
}

#[derive(Debug)]
struct ParsedRealityClientHello {
    raw_hello: Vec<u8>,
    random: [u8; 32],
    session_id: [u8; 32],
    peer_public_key: [u8; 32],
}

fn parse_client_key_share(data: &[u8]) -> io::Result<[u8; 32]> {
    if data.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid reality key share list",
        ));
    }
    let total_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + total_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated reality key share list",
        ));
    }

    let mut direct_x25519 = None;
    let mut hybrid_x25519 = None;
    let mut offset = 2;
    while offset + 4 <= 2 + total_len {
        let group = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let key_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;
        if offset + key_len > 2 + total_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated reality key share entry",
            ));
        }
        let key_data = &data[offset..offset + key_len];
        offset += key_len;
        match group {
            29 if key_data.len() == 32 => {
                let mut peer_public_key = [0u8; 32];
                peer_public_key.copy_from_slice(key_data);
                direct_x25519 = Some(peer_public_key);
            }
            4588 if key_data.len() >= 32 => {
                let mut peer_public_key = [0u8; 32];
                peer_public_key.copy_from_slice(&key_data[..32]);
                hybrid_x25519 = Some(peer_public_key);
            }
            _ => {}
        }
    }

    direct_x25519.or(hybrid_x25519).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "no supported reality key share found",
        )
    })
}

fn parse_reality_client_hello(
    record: &[u8],
    server_name: &str,
) -> io::Result<ParsedRealityClientHello> {
    if record.len() < 9 || record[0] != 0x16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid TLS record for reality handshake",
        ));
    }
    let record_len = u16::from_be_bytes([record[3], record[4]]) as usize;
    if record.len() != 5 + record_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated reality TLS record",
        ));
    }
    if record[5] != 0x01 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected TLS ClientHello",
        ));
    }
    let hello_len = ((record[6] as usize) << 16) | ((record[7] as usize) << 8) | record[8] as usize;
    if record.len() < 9 + hello_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated reality client hello",
        ));
    }
    let raw_hello = record[5..9 + hello_len].to_vec();
    let (_, client_hello) = parse_tls_handshake_client_hello(&record[9..9 + hello_len])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "failed to parse client hello"))?;
    let session_id = client_hello
        .session_id
        .filter(|sid| sid.len() == 32)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid reality session id"))?;
    let mut session_id_bytes = [0u8; 32];
    session_id_bytes.copy_from_slice(session_id);
    let mut random = [0u8; 32];
    random.copy_from_slice(client_hello.random);

    let extensions = parse_tls_client_hello_extensions(client_hello.ext.unwrap_or(&[]))
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "failed to parse client hello extensions",
            )
        })?
        .1;

    let mut sni = None;
    let mut peer_public_key = None;
    for ext in extensions {
        match ext {
            TlsExtension::SNI(names) => {
                for (name_type, data) in names {
                    if name_type == SNIType::HostName {
                        sni = std::str::from_utf8(data).ok().map(ToString::to_string);
                        break;
                    }
                }
            }
            TlsExtension::KeyShare(data) => {
                peer_public_key = Some(parse_client_key_share(data)?);
            }
            _ => {}
        }
    }

    if sni.as_deref() != Some(server_name) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "reality server name mismatch",
        ));
    }

    let mut raw_hello_zeroed = raw_hello;
    raw_hello_zeroed[39..71].fill(0);

    Ok(ParsedRealityClientHello {
        raw_hello: raw_hello_zeroed,
        random,
        session_id: session_id_bytes,
        peer_public_key: peer_public_key.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing reality key share extension",
            )
        })?,
    })
}

fn build_reality_cert(auth_key: &[u8; 32], server_name: &str) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let key_pair = KeyPair::generate_for(&PKCS_ED25519).map_err(map_other)?;
    let cert = CertificateParams::new(vec![server_name.to_string()])
        .map_err(map_other)?
        .self_signed(&key_pair)
        .map_err(map_other)?;

    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(auth_key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    mac.update(key_pair.public_key_raw());
    let signature = mac.finalize().into_bytes();

    let cert_der = yasna::parse_der(cert.der().as_ref(), |reader| {
        reader.read_sequence(|reader| {
            let tbs = reader.next().read_der()?;
            let alg = reader.next().read_der()?;
            let _ = reader.next().read_bitvec_bytes()?;
            Ok(yasna::construct_der(|writer| {
                writer.write_sequence(|writer| {
                    writer.next().write_der(&tbs);
                    writer.next().write_der(&alg);
                    writer
                        .next()
                        .write_bitvec_bytes(signature.as_ref(), signature.len() * 8);
                });
            }))
        })
    })
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    Ok((cert_der, key_pair.serialize_der()))
}

fn derive_reality_auth_key(
    cfg: &RealityServerConfig,
    parsed: &ParsedRealityClientHello,
) -> io::Result<[u8; 32]> {
    let private_key = StaticSecret::from(cfg.decode_private_key()?);
    let peer_public_key = X25519PublicKey::from(parsed.peer_public_key);
    let shared_secret = private_key.diffie_hellman(&peer_public_key);

    let hkdf = Hkdf::<Sha256>::new(Some(&parsed.random[..20]), shared_secret.as_bytes());
    let mut auth_key = [0u8; 32];
    hkdf.expand(b"REALITY", &mut auth_key)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "reality hkdf failed"))?;
    tracing::debug!("reality server auth_key prefix: {:02x?}", &auth_key[..16]);

    let cipher = Aes256Gcm::new(auth_key.as_ref().into());
    let mut decrypted = parsed.session_id.to_vec();
    cipher
        .decrypt_in_place(
            (&parsed.random[20..32]).into(),
            &parsed.raw_hello,
            &mut decrypted,
        )
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "invalid reality auth"))?;

    if decrypted.len() != 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid reality plaintext length",
        ));
    }

    let expected_short_id = cfg.decode_short_id()?;
    if decrypted[8..16] != expected_short_id {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid reality short id",
        ));
    }
    if let Some(max_time_diff) = cfg.max_time_diff {
        let client_time =
            u32::from_be_bytes([decrypted[4], decrypted[5], decrypted[6], decrypted[7]]);
        let client_time = UNIX_EPOCH + Duration::from_secs(client_time as u64);
        let diff = SystemTime::now()
            .duration_since(client_time)
            .unwrap_or_else(|e| e.duration());
        if diff > max_time_diff {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "reality client time diff too large",
            ));
        }
    }

    Ok(auth_key)
}

fn build_reality_server_config(
    cfg: &RealityServerConfig,
    auth_key: &[u8; 32],
) -> io::Result<Arc<ServerConfig>> {
    let (cert_der, key_der) = build_reality_cert(auth_key, &cfg.server_name)?;
    let provider = default_provider();
    let signing_key = provider
        .key_provider
        .load_private_key(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)))
        .map_err(map_other)
        .map_err(rd_interface::Error::to_io_err)?;
    let certified_key = RustlsCertifiedKey::new(
        vec![CertificateDer::from(cert_der)],
        Arc::new(RelaxedEd25519SigningKey(signing_key)),
    );
    let config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SingleCertAndKey::from(certified_key)));
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct DebugVerifier(Arc<dyn reality_rustls::client::danger::ServerCertVerifier>);

#[derive(Debug)]
struct RelaxedEd25519SigningKey(Arc<dyn RustlsSigningKey>);

impl RustlsSigningKey for RelaxedEd25519SigningKey {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn RustlsSigner>> {
        self.0.choose_scheme(offered).or_else(|| {
            (self.0.algorithm() == SignatureAlgorithm::ED25519)
                .then(|| self.0.choose_scheme(&[SignatureScheme::ED25519]))
                .flatten()
        })
    }

    fn public_key(&self) -> Option<reality_rustls::pki_types::SubjectPublicKeyInfoDer<'_>> {
        self.0.public_key()
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        self.0.algorithm()
    }
}

impl reality_rustls::client::danger::ServerCertVerifier for DebugVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &reality_rustls::pki_types::CertificateDer<'_>,
        intermediates: &[reality_rustls::pki_types::CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: reality_rustls::pki_types::UnixTime,
    ) -> std::result::Result<
        reality_rustls::client::danger::ServerCertVerified,
        reality_rustls::Error,
    > {
        self.0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &reality_rustls::pki_types::CertificateDer<'_>,
        dss: &reality_rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        reality_rustls::client::danger::HandshakeSignatureValid,
        reality_rustls::Error,
    > {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &reality_rustls::pki_types::CertificateDer<'_>,
        dss: &reality_rustls::DigitallySignedStruct,
    ) -> std::result::Result<
        reality_rustls::client::danger::HandshakeSignatureValid,
        reality_rustls::Error,
    > {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<reality_rustls::SignatureScheme> {
        self.0.supported_verify_schemes()
    }

    fn root_hint_subjects(&self) -> Option<&[reality_rustls::DistinguishedName]> {
        self.0.root_hint_subjects()
    }
}

fn create_reality_provider() -> Arc<reality_rustls::crypto::CryptoProvider> {
    let mut provider = default_provider();
    let mut new_kx_groups = vec![];
    for group in provider.kx_groups.iter() {
        if group.name() == reality_rustls::NamedGroup::X25519 {
            new_kx_groups
                .push(&X25519RealityGroup as &'static dyn reality_rustls::crypto::SupportedKxGroup);
        } else {
            new_kx_groups.push(*group);
        }
    }
    provider.kx_groups = new_kx_groups;
    Arc::new(provider)
}

pub(crate) fn build_rustls_config(cfg: &RealityConfig) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots))
        .build()
        .map_err(map_other)
        .map_err(rd_interface::Error::to_io_err)?;
    let reality_state = Arc::new(RealityConnectionState::new(
        cfg.decode_public_key()?,
        cfg.decode_short_id()?,
        Arc::new(DebugVerifier(verifier)),
    ));

    let mut config = ClientConfig::builder_with_provider(create_reality_provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(reality_state.clone())
        .with_no_client_auth();

    config.reality_callback = Some(reality_state);
    config.resumption = Resumption::disabled();
    config.alpn_protocols = vec![b"h2".to_vec().into(), b"http/1.1".to_vec().into()];

    Ok(Arc::new(config))
}

struct TlsBridge<'a, 'b, S> {
    stream: Pin<&'a mut S>,
    cx: &'a mut Context<'b>,
    safe_byte_read: bool,
}

impl<'a, 'b, S: AsyncRead> Read for TlsBridge<'a, 'b, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let read_len = if self.safe_byte_read { 1 } else { buf.len() };
        let mut read_buf = ReadBuf::new(&mut buf[..read_len]);
        match self.stream.as_mut().poll_read(self.cx, &mut read_buf) {
            Poll::Ready(Ok(())) => Ok(read_buf.filled().len()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }
}

impl<'a, 'b, S: AsyncWrite> Write for TlsBridge<'a, 'b, S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.stream.as_mut().poll_write(self.cx, buf) {
            Poll::Ready(Ok(n)) => Ok(n),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.stream.as_mut().poll_flush(self.cx) {
            Poll::Ready(Ok(())) => Ok(()),
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::Error::new(ErrorKind::WouldBlock, "WouldBlock")),
        }
    }
}

async fn read_tls_record<S>(stream: &mut S) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut header = [0u8; 5];
    tokio::io::AsyncReadExt::read_exact(stream, &mut header).await?;
    let record_len = u16::from_be_bytes([header[3], header[4]]) as usize;
    let mut record = vec![0u8; 5 + record_len];
    record[..5].copy_from_slice(&header);
    tokio::io::AsyncReadExt::read_exact(stream, &mut record[5..]).await?;
    Ok(record)
}

pub(crate) struct RealityServerStream<S> {
    conn: ServerConnection,
    stream: S,
    read_raw: bool,
    shared_read_raw: Option<Arc<AtomicBool>>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> RealityServerStream<S> {
    fn new(
        config: Arc<ServerConfig>,
        stream: S,
        shared_read_raw: Option<Arc<AtomicBool>>,
    ) -> io::Result<Self> {
        let conn = ServerConnection::new(config)
            .map_err(map_other)
            .map_err(rd_interface::Error::to_io_err)?;
        Ok(Self {
            conn,
            stream,
            read_raw: false,
            shared_read_raw,
        })
    }

    async fn perform_handshake(&mut self, initial_record: &[u8]) -> io::Result<()> {
        let mut initial_record = Some(initial_record.to_vec());
        std::future::poll_fn(|cx| {
            let mut progress = false;
            while self.conn.is_handshaking() {
                while self.conn.wants_write() {
                    let mut bridge = TlsBridge {
                        stream: Pin::new(&mut self.stream),
                        cx,
                        safe_byte_read: false,
                    };
                    match self.conn.write_tls(&mut bridge) {
                        Ok(n) if n > 0 => progress = true,
                        Ok(_) => break,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if self.conn.wants_read() {
                    let read_res = if let Some(initial) = initial_record.take() {
                        let mut slice = initial.as_slice();
                        let result = self.conn.read_tls(&mut slice);
                        if !slice.is_empty() {
                            initial_record = Some(slice.to_vec());
                        }
                        result
                    } else {
                        let mut bridge = TlsBridge {
                            stream: Pin::new(&mut self.stream),
                            cx,
                            safe_byte_read: false,
                        };
                        self.conn.read_tls(&mut bridge)
                    };
                    match read_res {
                        Ok(0) => {
                            return Poll::Ready(Err(io::Error::new(
                                ErrorKind::UnexpectedEof,
                                "connection closed during reality handshake",
                            )));
                        }
                        Ok(_) => {
                            if let Err(e) = self.conn.process_new_packets() {
                                return Poll::Ready(Err(io::Error::new(
                                    ErrorKind::InvalidData,
                                    format!("TLS Error: {e}"),
                                )));
                            }
                            progress = true;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if !progress {
                    return Poll::Pending;
                }
                progress = false;
            }
            Poll::Ready(Ok(()))
        })
        .await?;

        Ok(())
    }

    fn pump_read(&mut self, cx: &mut Context<'_>) -> io::Result<usize> {
        if self.conn.wants_read() {
            let mut bridge = TlsBridge {
                stream: Pin::new(&mut self.stream),
                cx,
                safe_byte_read: true,
            };
            match self.conn.read_tls(&mut bridge) {
                Ok(0) => return Err(io::Error::new(ErrorKind::UnexpectedEof, "EOF")),
                Ok(n) => {
                    self.conn.process_new_packets().map_err(|e| {
                        io::Error::new(ErrorKind::InvalidData, format!("TLS Error: {e}"))
                    })?;
                    return Ok(n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(0),
                Err(e) => return Err(e),
            }
        }
        Ok(0)
    }

    fn pump_write(&mut self, cx: &mut Context<'_>) -> io::Result<bool> {
        while self.conn.wants_write() {
            let mut bridge = TlsBridge {
                stream: Pin::new(&mut self.stream),
                cx,
                safe_byte_read: false,
            };
            match self.conn.write_tls(&mut bridge) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(false),
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RealityServerStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        let mut read_raw = this.read_raw;
        if !read_raw {
            if let Some(shared) = &this.shared_read_raw {
                read_raw = shared.load(Ordering::Relaxed);
                if read_raw {
                    this.read_raw = true;
                }
            }
        }
        if read_raw {
            return Pin::new(&mut this.stream).poll_read(cx, buf);
        }

        let _ = this.pump_write(cx)?;
        loop {
            let slice = buf.initialize_unfilled();
            match this.conn.reader().read(slice) {
                Ok(n) if n > 0 => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                _ => {
                    if this.conn.wants_read() {
                        let n = this.pump_read(cx)?;
                        if n == 0 {
                            return Poll::Pending;
                        }
                    } else if this.conn.wants_write() {
                        let _ = this.pump_write(cx)?;
                        return Poll::Pending;
                    } else {
                        return Poll::Ready(Ok(()));
                    }
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RealityServerStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let n = this.conn.writer().write(buf)?;
        let _ = this.pump_write(cx)?;
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.writer().flush()?;
        if !this.pump_write(cx)? {
            return Poll::Pending;
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.send_close_notify();
        let _ = this.pump_write(cx)?;
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

pub(crate) struct RealityStream<S> {
    conn: ClientConnection,
    stream: S,
    wr_buf: Vec<u8>,
    wr_pos: usize,
    read_raw: bool,
    shared_read_raw: Option<Arc<AtomicBool>>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> RealityStream<S> {
    pub(crate) fn new(
        config: Arc<ClientConfig>,
        name: ServerName<'static>,
        stream: S,
        shared_read_raw: Option<Arc<AtomicBool>>,
    ) -> io::Result<Self> {
        let conn = ClientConnection::new(config, name)
            .map_err(map_other)
            .map_err(rd_interface::Error::to_io_err)?;
        Ok(Self {
            conn,
            stream,
            wr_buf: Vec::with_capacity(4096),
            wr_pos: 0,
            read_raw: false,
            shared_read_raw,
        })
    }

    pub(crate) async fn perform_handshake(&mut self) -> io::Result<()> {
        std::future::poll_fn(|cx| {
            let mut progress = false;
            while self.conn.is_handshaking() {
                while self.conn.wants_write() {
                    let mut bridge = TlsBridge {
                        stream: Pin::new(&mut self.stream),
                        cx,
                        safe_byte_read: false,
                    };
                    match self.conn.write_tls(&mut bridge) {
                        Ok(n) if n > 0 => progress = true,
                        Ok(_) => break,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if self.conn.wants_read() {
                    let mut bridge = TlsBridge {
                        stream: Pin::new(&mut self.stream),
                        cx,
                        safe_byte_read: false,
                    };
                    match self.conn.read_tls(&mut bridge) {
                        Ok(0) => {
                            return Poll::Ready(Err(io::Error::new(
                                ErrorKind::UnexpectedEof,
                                "connection closed during reality handshake",
                            )));
                        }
                        Ok(_) => {
                            if let Err(e) = self.conn.process_new_packets() {
                                return Poll::Ready(Err(io::Error::new(
                                    ErrorKind::InvalidData,
                                    format!("TLS Error: {e}"),
                                )));
                            }
                            progress = true;
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                if !progress {
                    return Poll::Pending;
                }
                progress = false;
            }
            Poll::Ready(Ok(()))
        })
        .await?;

        self.flush_tls_output().await?;
        self.read_post_handshake_record().await?;
        self.drain_post_handshake_records().await?;
        Ok(())
    }

    async fn flush_tls_output(&mut self) -> io::Result<()> {
        let mut out = Vec::new();
        while self.conn.wants_write() {
            if self.conn.write_tls(&mut out)? == 0 {
                break;
            }
        }
        if !out.is_empty() {
            tokio::io::AsyncWriteExt::write_all(&mut self.stream, &out).await?;
            tokio::io::AsyncWriteExt::flush(&mut self.stream).await?;
        }
        Ok(())
    }

    async fn drain_post_handshake_records(&mut self) -> io::Result<()> {
        loop {
            let mut buf = [0u8; 8192];
            let Some(n) = std::future::poll_fn(|cx| -> Poll<io::Result<Option<usize>>> {
                let mut rb = ReadBuf::new(&mut buf);
                match Pin::new(&mut self.stream).poll_read(cx, &mut rb) {
                    Poll::Ready(Ok(())) => Poll::Ready(Ok(Some(rb.filled().len()))),
                    Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
                    Poll::Pending => Poll::Ready(Ok(None)),
                }
            })
            .await?
            else {
                break;
            };

            if n == 0 {
                break;
            }

            self.conn.read_tls(&mut Cursor::new(&buf[..n]))?;
            self.conn
                .process_new_packets()
                .map_err(|e| io::Error::new(ErrorKind::InvalidData, format!("TLS Error: {e}")))?;
            self.flush_tls_output().await?;
        }

        Ok(())
    }

    async fn read_post_handshake_record(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 8192];
        // Official Xray REALITY servers may emit a post-handshake TLS record slightly after
        // the client handshake loop completes. Waiting briefly here keeps interop stable.
        if let Ok(Ok(n)) = tokio::time::timeout(
            Duration::from_millis(100),
            tokio::io::AsyncReadExt::read(&mut self.stream, &mut buf),
        )
        .await
        {
            if n > 0 {
                self.conn.read_tls(&mut Cursor::new(&buf[..n]))?;
                self.conn.process_new_packets().map_err(|e| {
                    io::Error::new(ErrorKind::InvalidData, format!("TLS Error: {e}"))
                })?;
                self.flush_tls_output().await?;
            }
        }
        Ok(())
    }

    fn produce_tls_output(&mut self) -> io::Result<()> {
        if self.wr_pos == self.wr_buf.len() {
            self.wr_buf.clear();
            self.wr_pos = 0;
        }
        while self.conn.wants_write() {
            self.conn.write_tls(&mut self.wr_buf)?;
        }
        Ok(())
    }

    fn poll_flush_wr(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.wr_pos < self.wr_buf.len() {
            match Pin::new(&mut self.stream).poll_write(cx, &self.wr_buf[self.wr_pos..]) {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(io::Error::from(io::ErrorKind::WriteZero)));
                }
                Poll::Ready(Ok(n)) => self.wr_pos += n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        self.wr_buf.clear();
        self.wr_pos = 0;
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for RealityStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        let mut read_raw = this.read_raw;
        if !read_raw {
            if let Some(shared) = &this.shared_read_raw {
                read_raw = shared.load(Ordering::Relaxed);
                if read_raw {
                    this.read_raw = true;
                }
            }
        }
        if read_raw {
            return Pin::new(&mut this.stream).poll_read(cx, buf);
        }

        loop {
            let slice = buf.initialize_unfilled();
            match this.conn.reader().read(slice) {
                Ok(n) if n > 0 => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Ok(_) | Err(_) => {}
            }

            this.produce_tls_output()?;
            match this.poll_flush_wr(cx) {
                Poll::Ready(Ok(())) | Poll::Pending => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            }

            let mut tmp = [0u8; 4096];
            let mut rb = ReadBuf::new(&mut tmp);
            match Pin::new(&mut this.stream).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    let n = rb.filled().len();
                    if n == 0 {
                        return Poll::Ready(Ok(()));
                    }
                    this.conn.read_tls(&mut Cursor::new(rb.filled()))?;
                    this.conn.process_new_packets().map_err(|e| {
                        io::Error::new(ErrorKind::InvalidData, format!("TLS Error: {e}"))
                    })?;
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for RealityStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.wr_pos < this.wr_buf.len() {
            match this.poll_flush_wr(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = this.conn.writer().write(buf)?;
        this.produce_tls_output()?;
        let _ = this.poll_flush_wr(cx);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.writer().flush()?;
        this.produce_tls_output()?;
        match this.poll_flush_wr(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Pending => return Poll::Pending,
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.send_close_notify();
        this.produce_tls_output()?;
        match this.poll_flush_wr(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
            Poll::Pending => return Poll::Pending,
        }
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

pub(crate) async fn accept_reality_stream<S>(
    mut stream: S,
    cfg: &RealityServerConfig,
    shared_read_raw: Option<Arc<AtomicBool>>,
) -> io::Result<RealityServerStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let record = read_tls_record(&mut stream).await?;
    let config = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        let record = record.clone();
        move || {
            let parsed = parse_reality_client_hello(&record, &cfg.server_name)?;
            let auth_key = derive_reality_auth_key(&cfg, &parsed)?;
            build_reality_server_config(&cfg, &auth_key)
        }
    })
    .await
    .map_err(|e| io::Error::other(format!("reality handshake worker failed: {e}")))??;
    let mut reality = RealityServerStream::new(config, stream, shared_read_raw)?;
    reality.perform_handshake(&record).await?;
    Ok(reality)
}

pub(crate) async fn connect_reality_stream<S>(
    stream: S,
    config: Arc<ClientConfig>,
    server_name: String,
    shared_read_raw: Option<Arc<AtomicBool>>,
) -> io::Result<RealityStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let server_name = ServerName::try_from(server_name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut reality = RealityStream::new(config, server_name, stream, shared_read_raw)?;
    reality.perform_handshake().await?;
    Ok(reality)
}

#[cfg(test)]
mod tests {
    use super::parse_client_key_share;

    fn encode_key_share(group: u16, key_data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let total_len = 4 + key_data.len();
        out.extend_from_slice(&(total_len as u16).to_be_bytes());
        out.extend_from_slice(&group.to_be_bytes());
        out.extend_from_slice(&(key_data.len() as u16).to_be_bytes());
        out.extend_from_slice(key_data);
        out
    }

    #[test]
    fn test_parse_client_key_share_prefers_direct_x25519() {
        let direct = [0x11; 32];
        let hybrid = [0x22; 64];
        let mut buf = Vec::new();
        let total_len = (4 + hybrid.len()) + (4 + direct.len());
        buf.extend_from_slice(&(total_len as u16).to_be_bytes());
        buf.extend_from_slice(&4588u16.to_be_bytes());
        buf.extend_from_slice(&(hybrid.len() as u16).to_be_bytes());
        buf.extend_from_slice(&hybrid);
        buf.extend_from_slice(&29u16.to_be_bytes());
        buf.extend_from_slice(&(direct.len() as u16).to_be_bytes());
        buf.extend_from_slice(&direct);

        assert_eq!(parse_client_key_share(&buf).unwrap(), direct);
    }

    #[test]
    fn test_parse_client_key_share_falls_back_to_hybrid_prefix() {
        let hybrid = [0x33; 64];
        let buf = encode_key_share(4588, &hybrid);

        assert_eq!(parse_client_key_share(&buf).unwrap(), [0x33; 32]);
    }
}
