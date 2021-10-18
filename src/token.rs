use ct_codecs::{Base64UrlSafeNoPadding, Decoder, Encoder, Hex};
use serde::{de::DeserializeOwned, Serialize};

use crate::claims::*;
use crate::common::*;
use crate::error::*;
use crate::jwt_header::*;

pub const MAX_HEADER_LENGTH: usize = 8192;

/// Utilities to get information about a JWT token
pub struct Token;

/// JWT token information useful before signature/tag verification
#[derive(Debug, Clone, Default)]
pub struct TokenMetadata {
    jwt_header: JWTHeader,
}

impl TokenMetadata {
    /// The JWT algorithm for this token
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    /// Clients should ignore it and use the correct type of key directly.
    pub fn algorithm(&self) -> &str {
        &self.jwt_header.algorithm
    }

    /// The content type for this token
    pub fn content_type(&self) -> Option<&str> {
        self.jwt_header.content_type.as_deref()
    }

    /// The key, or public key identifier for this token
    pub fn key_id(&self) -> Option<&str> {
        self.jwt_header.key_id.as_deref()
    }

    /// The signature type for this token
    pub fn signature_type(&self) -> Option<&str> {
        self.jwt_header.signature_type.as_deref()
    }

    /// The set of raw critical properties for this token
    pub fn critical(&self) -> Option<&[String]> {
        self.jwt_header.critical.as_deref()
    }

    /// The certificate chain for this token
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    pub fn certificate_chain(&self) -> Option<&[String]> {
        self.jwt_header.certificate_chain.as_deref()
    }

    /// The key set URL for this token
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    /// At the bare minimum, you should check that the URL belongs to the domain you expect.
    pub fn key_set_url(&self) -> Option<&str> {
        self.jwt_header.key_set_url.as_deref()
    }

    /// The public key for this token
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    /// At the bare minimum, you should check that it's in a set of public keys you already trust.
    pub fn public_key(&self) -> Option<&str> {
        self.jwt_header.public_key.as_deref()
    }

    /// The certificate URL for this token.
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    /// At the bare minimum, you should check that the URL belongs to the domain you expect.
    pub fn certificate_url(&self) -> Option<&str> {
        self.jwt_header.certificate_url.as_deref()
    }

    /// URLsafe-base64-encoded SHA1 hash of the X.509 certificate for this token.
    /// In practice, it can also be any string representing the public key.
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    pub fn certificate_sha1_thumbprint(&self) -> Option<&str> {
        self.jwt_header.certificate_sha1_thumbprint.as_deref()
    }

    /// URLsafe-base64-encoded SHA256 hash of the X.509 certificate for this token.
    /// In practice, it can also be any string representing the public key.
    /// This information should not be trusted: it is unprotected and can be freely modified by a third party.
    pub fn certificate_sha256_thumbprint(&self) -> Option<&str> {
        self.jwt_header.certificate_sha256_thumbprint.as_deref()
    }
}

impl Token {
    pub(crate) fn build<AuthenticationOrSignatureFn, CustomClaims: Serialize + DeserializeOwned>(
        jwt_header: &JWTHeader,
        claims: JWTClaims<CustomClaims>,
        authentication_or_signature_fn: AuthenticationOrSignatureFn,
    ) -> Result<String, Error>
    where
        AuthenticationOrSignatureFn: FnOnce(&str) -> Result<Vec<u8>, Error>,
    {
        let jwt_header_json = serde_json::to_string(&jwt_header)?;
        let claims_json = serde_json::to_string(&claims)?;
        let authenticated = format!(
            "{}.{}",
            Base64UrlSafeNoPadding::encode_to_string(jwt_header_json)?,
            Base64UrlSafeNoPadding::encode_to_string(claims_json)?
        );
        let authentication_tag_or_signature = authentication_or_signature_fn(&authenticated)?;
        let mut token = authenticated;
        token.push('.');
        token.push_str(&Base64UrlSafeNoPadding::encode_to_string(
            &authentication_tag_or_signature,
        )?);
        Ok(token)
    }

    pub(crate) fn verify<AuthenticationOrSignatureFn, CustomClaims: Serialize + DeserializeOwned>(
        jwt_alg_name: &'static str,
        token: &str,
        options: Option<VerificationOptions>,
        authentication_or_signature_fn: AuthenticationOrSignatureFn,
    ) -> Result<JWTClaims<CustomClaims>, Error>
    where
        AuthenticationOrSignatureFn: FnOnce(&str, &[u8]) -> Result<(), Error>,
    {
        let options = options.unwrap_or_default();
        let mut parts = token.split('.');
        let jwt_header_b64 = parts.next().ok_or(JWTError::CompactEncodingError)?;
        ensure!(
            jwt_header_b64.len() <= MAX_HEADER_LENGTH,
            JWTError::HeaderTooLarge
        );
        let claims_b64 = parts.next().ok_or(JWTError::CompactEncodingError)?;
        let authentication_tag_b64 = parts.next().ok_or(JWTError::CompactEncodingError)?;
        ensure!(parts.next().is_none(), JWTError::CompactEncodingError);
        let jwt_header: JWTHeader = serde_json::from_slice(
            &Base64UrlSafeNoPadding::decode_to_vec(jwt_header_b64, None)?,
        )?;
        if let Some(signature_type) = &jwt_header.signature_type {
            ensure!(signature_type == "JWT", JWTError::NotJWT);
        }
        ensure!(
            jwt_header.algorithm == jwt_alg_name,
            JWTError::AlgorithmMismatch
        );
        if let Some(required_key_id) = &options.required_key_id {
            if let Some(key_id) = &jwt_header.key_id {
                ensure!(key_id == required_key_id, JWTError::KeyIdentifierMismatch);
            } else {
                bail!(JWTError::MissingJWTKeyIdentifier)
            }
        }
        let authentication_tag =
            Base64UrlSafeNoPadding::decode_to_vec(&authentication_tag_b64, None)?;
        let authenticated = &token[..jwt_header_b64.len() + 1 + claims_b64.len()];
        authentication_or_signature_fn(authenticated, &authentication_tag)?;
        let claims: JWTClaims<CustomClaims> =
            serde_json::from_slice(&Base64UrlSafeNoPadding::decode_to_vec(&claims_b64, None)?)?;
        claims.validate(&options)?;
        Ok(claims)
    }

    /// Decode token information that can be usedful prior to signature/tag verification
    pub fn decode_metadata(token: &str) -> Result<TokenMetadata, Error> {
        let mut parts = token.split('.');
        let jwt_header_b64 = parts.next().ok_or(JWTError::CompactEncodingError)?;
        ensure!(
            jwt_header_b64.len() <= MAX_HEADER_LENGTH,
            JWTError::HeaderTooLarge
        );
        let jwt_header: JWTHeader = serde_json::from_slice(
            &Base64UrlSafeNoPadding::decode_to_vec(jwt_header_b64, None)?,
        )?;
        Ok(TokenMetadata { jwt_header })
    }
}

/// Unsigned metadata to be attached to a new token
#[derive(Debug, Clone, Default)]
pub struct NewTokenMetadata {
    pub(crate) jwt_header: JWTHeader,
}

impl NewTokenMetadata {
    pub(crate) fn new(algorithm: String, key_id: Option<String>) -> Self {
        let jwt_header = JWTHeader {
            algorithm,
            key_id,
            ..Default::default()
        };
        Self { jwt_header }
    }

    pub fn with_key_set_url(mut self, key_set_url: impl ToString) -> Self {
        self.jwt_header.key_set_url = Some(key_set_url.to_string());
        self
    }

    pub fn with_public_key(mut self, public_key: impl ToString) -> Self {
        self.jwt_header.public_key = Some(public_key.to_string());
        self
    }

    pub fn with_certificate_url(mut self, certificate_url: impl ToString) -> Self {
        self.jwt_header.certificate_url = Some(certificate_url.to_string());
        self
    }

    pub fn with_certificate_sha1_thumbprint(
        mut self,
        certificate_sha1_thumbprint: impl ToString,
    ) -> Result<Self, Error> {
        let thumbprint = certificate_sha1_thumbprint.to_string();
        let mut bin = [0u8; 20];
        if thumbprint.len() == 40 {
            ensure!(
                Hex::decode(&mut bin, &thumbprint, None)?.len() == bin.len(),
                JWTError::InvalidCertThumprint
            );
            let thumbprint = Base64UrlSafeNoPadding::encode_to_string(&bin)?;
            self.jwt_header.certificate_sha1_thumbprint = Some(thumbprint);
            return Ok(self);
        }
        ensure!(
            Base64UrlSafeNoPadding::decode(&mut bin, &thumbprint, None)?.len() == bin.len(),
            JWTError::InvalidCertThumprint
        );
        self.jwt_header.certificate_sha1_thumbprint = Some(thumbprint);
        Ok(self)
    }

    pub fn with_certificate_sha256_thumbprint(
        mut self,
        certificate_sha256_thumbprint: impl ToString,
    ) -> Result<Self, Error> {
        let thumbprint = certificate_sha256_thumbprint.to_string();
        let mut bin = [0u8; 32];
        if thumbprint.len() == 64 {
            ensure!(
                Hex::decode(&mut bin, &thumbprint, None)?.len() == bin.len(),
                JWTError::InvalidCertThumprint
            );
            let thumbprint = Base64UrlSafeNoPadding::encode_to_string(&bin)?;
            self.jwt_header.certificate_sha1_thumbprint = Some(thumbprint);
            return Ok(self);
        }
        ensure!(
            Base64UrlSafeNoPadding::decode(&mut bin, &thumbprint, None)?.len() == bin.len(),
            JWTError::InvalidCertThumprint
        );
        self.jwt_header.certificate_sha1_thumbprint = Some(thumbprint);
        Ok(self)
    }
}

#[test]
fn should_verify_token() {
    use crate::prelude::*;

    let key = HS256Key::generate();

    let issuer = "issuer";
    let audience = "recipient";
    let mut claims = Claims::create(Duration::from_mins(10))
        .with_issuer(issuer)
        .with_audience(audience);
    let nonce = claims.create_nonce();
    let token = key.authenticate(claims).unwrap();

    let options = VerificationOptions {
        required_nonce: Some(nonce),
        allowed_issuers: Some(HashSet::from_strings(&[issuer])),
        allowed_audiences: Some(HashSet::from_strings(&[audience])),
        ..Default::default()
    };
    key.verify_token::<NoCustomClaims>(&token, Some(options))
        .unwrap();
}

#[test]
fn multiple_audiences() {
    use crate::prelude::*;
    use std::collections::HashSet;

    let key = HS256Key::generate();

    let mut audiences = HashSet::new();
    audiences.insert("audience 1");
    audiences.insert("audience 2");
    audiences.insert("audience 3");
    let claims = Claims::create(Duration::from_mins(10)).with_audiences(audiences);
    let token = key.authenticate(claims).unwrap();

    let options = VerificationOptions {
        allowed_audiences: Some(HashSet::from_strings(&["audience 1"])),
        ..Default::default()
    };
    key.verify_token::<NoCustomClaims>(&token, Some(options))
        .unwrap();
}

#[test]
fn explicitly_empty_audiences() {
    use crate::prelude::*;
    use std::collections::HashSet;

    let key = HS256Key::generate();

    let audiences: HashSet<&str> = HashSet::new();
    let claims = Claims::create(Duration::from_mins(10)).with_audiences(audiences);
    let token = key.authenticate(claims).unwrap();
    let decoded = key.verify_token::<NoCustomClaims>(&token, None).unwrap();
    assert!(decoded.audiences.is_some());

    let claims = Claims::create(Duration::from_mins(10)).with_audience("");
    let token = key.authenticate(claims).unwrap();
    let decoded = key.verify_token::<NoCustomClaims>(&token, None).unwrap();
    assert!(decoded.audiences.is_some());

    let claims = Claims::create(Duration::from_mins(10));
    let token = key.authenticate(claims).unwrap();
    let decoded = key.verify_token::<NoCustomClaims>(&token, None).unwrap();
    assert!(decoded.audiences.is_none());
}
