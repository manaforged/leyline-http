use std::fmt::Write;

use crate::asn1::Asn1Object;
use crate::error::ErrorStack;
use crate::nid::Nid;
use crate::x509::{GeneralName, Stack, X509Extension, X509v3Context};
use foreign_types::ForeignType;

pub struct BasicConstraints {
    critical: bool,
    ca: bool,
    pathlen: Option<u32>,
}

impl Default for BasicConstraints {
    fn default() -> BasicConstraints {
        BasicConstraints::new()
    }
}

impl BasicConstraints {
    #[must_use]
    pub fn new() -> BasicConstraints {
        BasicConstraints {
            critical: false,
            ca: false,
            pathlen: None,
        }
    }

    pub fn critical(&mut self) -> &mut BasicConstraints {
        self.critical = true;
        self
    }

    pub fn ca(&mut self) -> &mut BasicConstraints {
        self.ca = true;
        self
    }

    pub fn pathlen(&mut self, pathlen: u32) -> &mut BasicConstraints {
        self.pathlen = Some(pathlen);
        self
    }

    pub fn build(&self) -> Result<X509Extension, ErrorStack> {
        let mut value = String::new();
        if self.critical {
            value.push_str("critical,");
        }
        value.push_str("CA:");
        if self.ca {
            value.push_str("TRUE");
        } else {
            value.push_str("FALSE");
        }
        if let Some(pathlen) = self.pathlen {
            write!(value, ",pathlen:{pathlen}").map_err(ErrorStack::internal_error)?;
        }
        X509Extension::new_nid(None, None, Nid::BASIC_CONSTRAINTS, &value)
    }
}

pub struct KeyUsage {
    critical: bool,
    digital_signature: bool,
    non_repudiation: bool,
    key_encipherment: bool,
    data_encipherment: bool,
    key_agreement: bool,
    key_cert_sign: bool,
    crl_sign: bool,
    encipher_only: bool,
    decipher_only: bool,
}

impl Default for KeyUsage {
    fn default() -> KeyUsage {
        KeyUsage::new()
    }
}

impl KeyUsage {
    #[must_use]
    pub fn new() -> KeyUsage {
        KeyUsage {
            critical: false,
            digital_signature: false,
            non_repudiation: false,
            key_encipherment: false,
            data_encipherment: false,
            key_agreement: false,
            key_cert_sign: false,
            crl_sign: false,
            encipher_only: false,
            decipher_only: false,
        }
    }

    pub fn critical(&mut self) -> &mut KeyUsage {
        self.critical = true;
        self
    }

    pub fn digital_signature(&mut self) -> &mut KeyUsage {
        self.digital_signature = true;
        self
    }

    pub fn non_repudiation(&mut self) -> &mut KeyUsage {
        self.non_repudiation = true;
        self
    }

    pub fn key_encipherment(&mut self) -> &mut KeyUsage {
        self.key_encipherment = true;
        self
    }

    pub fn data_encipherment(&mut self) -> &mut KeyUsage {
        self.data_encipherment = true;
        self
    }

    pub fn key_agreement(&mut self) -> &mut KeyUsage {
        self.key_agreement = true;
        self
    }

    pub fn key_cert_sign(&mut self) -> &mut KeyUsage {
        self.key_cert_sign = true;
        self
    }

    pub fn crl_sign(&mut self) -> &mut KeyUsage {
        self.crl_sign = true;
        self
    }

    pub fn encipher_only(&mut self) -> &mut KeyUsage {
        self.encipher_only = true;
        self
    }

    pub fn decipher_only(&mut self) -> &mut KeyUsage {
        self.decipher_only = true;
        self
    }

    pub fn build(&self) -> Result<X509Extension, ErrorStack> {
        let mut value = String::new();
        let mut first = true;
        append(&mut value, &mut first, self.critical, "critical");
        append(
            &mut value,
            &mut first,
            self.digital_signature,
            "digitalSignature",
        );
        append(
            &mut value,
            &mut first,
            self.non_repudiation,
            "nonRepudiation",
        );
        append(
            &mut value,
            &mut first,
            self.key_encipherment,
            "keyEncipherment",
        );
        append(
            &mut value,
            &mut first,
            self.data_encipherment,
            "dataEncipherment",
        );
        append(&mut value, &mut first, self.key_agreement, "keyAgreement");
        append(&mut value, &mut first, self.key_cert_sign, "keyCertSign");
        append(&mut value, &mut first, self.crl_sign, "cRLSign");
        append(&mut value, &mut first, self.encipher_only, "encipherOnly");
        append(&mut value, &mut first, self.decipher_only, "decipherOnly");
        X509Extension::new_nid(None, None, Nid::KEY_USAGE, &value)
    }
}

pub struct ExtendedKeyUsage {
    critical: bool,
    items: Vec<String>,
}

impl Default for ExtendedKeyUsage {
    fn default() -> ExtendedKeyUsage {
        ExtendedKeyUsage::new()
    }
}

impl ExtendedKeyUsage {
    #[must_use]
    pub fn new() -> ExtendedKeyUsage {
        ExtendedKeyUsage {
            critical: false,
            items: vec![],
        }
    }

    pub fn critical(&mut self) -> &mut ExtendedKeyUsage {
        self.critical = true;
        self
    }

    pub fn server_auth(&mut self) -> &mut ExtendedKeyUsage {
        self.other("serverAuth")
    }

    pub fn client_auth(&mut self) -> &mut ExtendedKeyUsage {
        self.other("clientAuth")
    }

    pub fn code_signing(&mut self) -> &mut ExtendedKeyUsage {
        self.other("codeSigning")
    }

    pub fn time_stamping(&mut self) -> &mut ExtendedKeyUsage {
        self.other("timeStamping")
    }

    pub fn ms_code_ind(&mut self) -> &mut ExtendedKeyUsage {
        self.other("msCodeInd")
    }

    pub fn ms_code_com(&mut self) -> &mut ExtendedKeyUsage {
        self.other("msCodeCom")
    }

    pub fn ms_ctl_sign(&mut self) -> &mut ExtendedKeyUsage {
        self.other("msCTLSign")
    }

    pub fn ms_sgc(&mut self) -> &mut ExtendedKeyUsage {
        self.other("msSGC")
    }

    pub fn ms_efs(&mut self) -> &mut ExtendedKeyUsage {
        self.other("msEFS")
    }

    pub fn ns_sgc(&mut self) -> &mut ExtendedKeyUsage {
        self.other("nsSGC")
    }

    pub fn other(&mut self, other: &str) -> &mut ExtendedKeyUsage {
        self.items.push(other.to_string());
        self
    }

    pub fn build(&self) -> Result<X509Extension, ErrorStack> {
        let mut stack = Stack::new()?;
        for item in &self.items {
            stack.push(Asn1Object::from_str(item)?)?;
        }
        unsafe {
            X509Extension::new_internal(Nid::EXT_KEY_USAGE, self.critical, stack.as_ptr().cast())
        }
    }
}

pub struct SubjectKeyIdentifier {
    critical: bool,
}

impl Default for SubjectKeyIdentifier {
    fn default() -> SubjectKeyIdentifier {
        SubjectKeyIdentifier::new()
    }
}

impl SubjectKeyIdentifier {
    #[must_use]
    pub fn new() -> SubjectKeyIdentifier {
        SubjectKeyIdentifier { critical: false }
    }

    pub fn critical(&mut self) -> &mut SubjectKeyIdentifier {
        self.critical = true;
        self
    }

    pub fn build(&self, ctx: &X509v3Context) -> Result<X509Extension, ErrorStack> {
        let mut value = String::new();
        let mut first = true;
        append(&mut value, &mut first, self.critical, "critical");
        append(&mut value, &mut first, true, "hash");
        X509Extension::new_nid(None, Some(ctx), Nid::SUBJECT_KEY_IDENTIFIER, &value)
    }
}

pub struct AuthorityKeyIdentifier {
    critical: bool,
    keyid: Option<bool>,
    issuer: Option<bool>,
}

impl Default for AuthorityKeyIdentifier {
    fn default() -> AuthorityKeyIdentifier {
        AuthorityKeyIdentifier::new()
    }
}

impl AuthorityKeyIdentifier {
    #[must_use]
    pub fn new() -> AuthorityKeyIdentifier {
        AuthorityKeyIdentifier {
            critical: false,
            keyid: None,
            issuer: None,
        }
    }

    pub fn critical(&mut self) -> &mut AuthorityKeyIdentifier {
        self.critical = true;
        self
    }

    pub fn keyid(&mut self, always: bool) -> &mut AuthorityKeyIdentifier {
        self.keyid = Some(always);
        self
    }

    pub fn issuer(&mut self, always: bool) -> &mut AuthorityKeyIdentifier {
        self.issuer = Some(always);
        self
    }

    pub fn build(&self, ctx: &X509v3Context) -> Result<X509Extension, ErrorStack> {
        let mut value = String::new();
        let mut first = true;
        append(&mut value, &mut first, self.critical, "critical");
        match self.keyid {
            Some(true) => append(&mut value, &mut first, true, "keyid:always"),
            Some(false) => append(&mut value, &mut first, true, "keyid"),
            None => {}
        }
        match self.issuer {
            Some(true) => append(&mut value, &mut first, true, "issuer:always"),
            Some(false) => append(&mut value, &mut first, true, "issuer"),
            None => {}
        }
        X509Extension::new_nid(None, Some(ctx), Nid::AUTHORITY_KEY_IDENTIFIER, &value)
    }
}

enum RustGeneralName {
    Dns(String),
    Email(String),
    Uri(String),
    Ip(String),
    Rid(String),
}

pub struct SubjectAlternativeName {
    critical: bool,
    items: Vec<RustGeneralName>,
}

impl Default for SubjectAlternativeName {
    fn default() -> SubjectAlternativeName {
        SubjectAlternativeName::new()
    }
}

impl SubjectAlternativeName {
    #[must_use]
    pub fn new() -> SubjectAlternativeName {
        SubjectAlternativeName {
            critical: false,
            items: vec![],
        }
    }

    pub fn critical(&mut self) -> &mut SubjectAlternativeName {
        self.critical = true;
        self
    }

    pub fn email(&mut self, email: &str) -> &mut SubjectAlternativeName {
        self.items.push(RustGeneralName::Email(email.to_owned()));
        self
    }

    pub fn uri(&mut self, uri: &str) -> &mut SubjectAlternativeName {
        self.items.push(RustGeneralName::Uri(uri.to_owned()));
        self
    }

    pub fn dns(&mut self, dns: &str) -> &mut SubjectAlternativeName {
        self.items.push(RustGeneralName::Dns(dns.to_owned()));
        self
    }

    pub fn rid(&mut self, rid: &str) -> &mut SubjectAlternativeName {
        self.items.push(RustGeneralName::Rid(rid.to_owned()));
        self
    }

    pub fn ip(&mut self, ip: &str) -> &mut SubjectAlternativeName {
        self.items.push(RustGeneralName::Ip(ip.to_owned()));
        self
    }

    #[deprecated = "dir_name is deprecated and always panics. Please file a bug if you have a use case for this."]
    pub fn dir_name(&mut self, _dir_name: &str) -> &mut SubjectAlternativeName {
        unimplemented!(
            "This has not yet been adapted for the new internals. File a bug if you need this."
        );
    }

    #[deprecated = "other_name is deprecated and always panics. Please file a bug if you have a use case for this."]
    pub fn other_name(&mut self, _other_name: &str) -> &mut SubjectAlternativeName {
        unimplemented!(
            "This has not yet been adapted for the new internals. File a bug if you need this."
        );
    }

    pub fn build(&self, _ctx: &X509v3Context<'_>) -> Result<X509Extension, ErrorStack> {
        let mut stack = Stack::new()?;
        for item in &self.items {
            let gn = match item {
                RustGeneralName::Dns(s) => GeneralName::new_dns(s.as_bytes())?,
                RustGeneralName::Email(s) => GeneralName::new_email(s.as_bytes())?,
                RustGeneralName::Uri(s) => GeneralName::new_uri(s.as_bytes())?,
                RustGeneralName::Ip(s) => {
                    GeneralName::new_ip(s.parse().map_err(|_| ErrorStack::get())?)?
                }
                RustGeneralName::Rid(s) => GeneralName::new_rid(Asn1Object::from_str(s)?)?,
            };
            stack.push(gn)?;
        }

        unsafe {
            X509Extension::new_internal(Nid::SUBJECT_ALT_NAME, self.critical, stack.as_ptr().cast())
        }
    }
}

fn append(value: &mut String, first: &mut bool, should: bool, element: &str) {
    if !should {
        return;
    }

    if !*first {
        value.push(',');
    }
    *first = false;
    value.push_str(element);
}
