//! 私钥的解析和生成，密钥管理和登录共用。

use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
use russh::keys::{Algorithm, EcdsaCurve, HashAlg, PrivateKey};

use crate::i18n::Msg;

/// 私钥文本的长度上限，超过的肯定不是私钥。
pub const MAX_KEY_SIZE: usize = 64 * 1024;
/// 生成 RSA 密钥可选的位数。
pub const RSA_BITS: [u32; 3] = [2048, 3072, 4096];

/// 私钥的公开信息。
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// 如 ssh-ed25519、ssh-rsa、ecdsa-sha2-nistp256。
    pub algorithm: String,
    /// RSA 密钥的位数。
    pub bits: Option<u32>,
    /// SHA256:xxxx
    pub fingerprint: String,
    /// authorized_keys 格式的一行公钥。
    pub public_key: String,
    pub comment: String,
}

/// 解析私钥文本（OpenSSH、PEM 或 PuTTY ppk），加密的私钥用 passphrase 解开。
/// 返回私钥和是否用到了密码。
pub fn decode(text: &str, passphrase: &str) -> Result<(PrivateKey, bool), Msg> {
    use russh::keys::{Error, decode_secret_key};
    let parse_failed = |e: Error| msg!("err.ssh.keyParseFailed", "无法解析私钥").with_cause(e);
    match decode_secret_key(text, None) {
        Ok(k) => Ok((k, false)),
        Err(Error::KeyIsEncrypted) if passphrase.is_empty() => Err(msg!(
            "err.ssh.keyEncrypted",
            "私钥已加密，需要填写私钥密码"
        )),
        Err(Error::KeyIsEncrypted) => decode_secret_key(text, Some(passphrase))
            .map(|k| (k, true))
            .map_err(|e| msg!("err.ssh.keyDecryptFailed", "解密私钥失败（密码错误？）").with_cause(e)),
        // 加密的 ppk 不带密码时报的是一般的解析错误，填了密码就再试一次。
        Err(e) if !passphrase.is_empty() => decode_secret_key(text, Some(passphrase))
            .map(|k| (k, true))
            .map_err(|_| parse_failed(e)),
        Err(e) => Err(parse_failed(e)),
    }
}

pub fn info(key: &PrivateKey) -> KeyInfo {
    let public = key.public_key();
    KeyInfo {
        algorithm: key.algorithm().as_str().to_string(),
        bits: public.key_data().rsa().map(|k| k.key_size()),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        public_key: public.to_openssh().unwrap_or_default(),
        comment: key.comment().as_str_lossy().to_string(),
    }
}

/// 要生成的密钥类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    /// NIST P-256
    Ecdsa,
    Rsa(u32),
}

/// 生成新的私钥，返回 OpenSSH 格式的文本；passphrase 不为空时加密。RSA 位数较大时要几秒钟。
pub fn generate(
    kind: KeyKind,
    comment: &str,
    passphrase: &str,
) -> Result<String, russh::keys::ssh_key::Error> {
    let mut rng = russh::keys::key::safe_rng();
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
        KeyKind::Ecdsa => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )?,
        KeyKind::Rsa(bits) => {
            let pair = RsaKeypair::random(&mut rng, bits as usize)?;
            PrivateKey::new(KeypairData::from(pair), "")?
        }
    };
    key.set_comment(comment);
    if !passphrase.is_empty() {
        key = key.encrypt(&mut rng, passphrase)?;
    }
    Ok(key.to_openssh(LineEnding::LF)?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_decode() {
        let text = generate(KeyKind::Ed25519, "me@pc", "").unwrap();
        let (key, encrypted) = decode(&text, "ignored").unwrap();
        assert!(!encrypted, "没加密的私钥不需要密码");
        let i = info(&key);
        assert_eq!(i.algorithm, "ssh-ed25519");
        assert_eq!(i.bits, None);
        assert!(i.fingerprint.starts_with("SHA256:"));
        assert!(i.public_key.starts_with("ssh-ed25519 ") && i.public_key.ends_with(" me@pc"));
        assert_eq!(i.comment, "me@pc");

        let text = generate(KeyKind::Ecdsa, "", "s3cret").unwrap();
        assert_eq!(decode(&text, "").unwrap_err().i18n, "err.ssh.keyEncrypted");
        assert_eq!(
            decode(&text, "wrong").unwrap_err().i18n,
            "err.ssh.keyDecryptFailed"
        );
        let (key, encrypted) = decode(&text, "s3cret").unwrap();
        assert!(encrypted);
        assert_eq!(info(&key).algorithm, "ecdsa-sha2-nistp256");

        assert_eq!(
            decode("not a key", "").unwrap_err().i18n,
            "err.ssh.keyParseFailed"
        );
    }
}
