use std::io;

use leyline_bssl::asn1::Asn1Time;
use leyline_bssl::bn::{BigNum, MsbOption};
use leyline_bssl::ec::{EcGroup, EcKey};
use leyline_bssl::error::ErrorStack;
use leyline_bssl::hash::MessageDigest;
use leyline_bssl::nid::Nid;
use leyline_bssl::pkey::{PKey, Private};
use leyline_bssl::ssl::{AlpnError, SslAcceptor, SslMethod, select_next_proto};
use leyline_bssl::x509::extension::{BasicConstraints, ExtendedKeyUsage, SubjectAlternativeName};
use leyline_bssl::x509::{X509, X509Builder, X509Name, X509NameBuilder};

const CA_COMMON_NAME: &str = "Test Server Root CA";
const LEAF_COMMON_NAME: &str = "Test Server";
const LEAF_DNS_NAMES: &[&str] = &["localhost"];
const LEAF_IP_ADDRESSES: &[&str] = &["127.0.0.1"];
const VALID_DAYS: u32 = 2;
const SERIAL_BITS: i32 = 159;

pub(super) struct TestTls {
    pub(super) acceptor: SslAcceptor,
    pub(super) ca_der: Vec<u8>,
}

pub(super) fn build() -> io::Result<TestTls> {
    build_inner().map_err(io::Error::other)
}

fn build_inner() -> Result<TestTls, ErrorStack> {
    let ca_key = new_key()?;
    let ca = ca_cert(&ca_key)?;
    let leaf_key = new_key()?;
    let leaf = leaf_cert(&ca, &ca_key, &leaf_key)?;
    let ca_der = ca.to_der()?;
    Ok(TestTls {
        acceptor: acceptor(&leaf_key, &leaf, ca)?,
        ca_der,
    })
}

fn ca_cert(ca_key: &PKey<Private>) -> Result<X509, ErrorStack> {
    let ca_name = name(CA_COMMON_NAME)?;
    let mut ca = base_cert(&ca_name, &ca_name, ca_key)?;
    let constraints = BasicConstraints::new().critical().ca().build()?;
    ca.append_extension(&constraints)?;
    ca.sign(ca_key, MessageDigest::sha256())?;
    Ok(ca.build())
}

fn leaf_cert(
    ca: &X509,
    ca_key: &PKey<Private>,
    leaf_key: &PKey<Private>,
) -> Result<X509, ErrorStack> {
    let mut leaf = base_cert(&name(LEAF_COMMON_NAME)?, ca.subject_name(), leaf_key)?;
    let mut san = SubjectAlternativeName::new();
    for dns in LEAF_DNS_NAMES {
        san.dns(dns);
    }
    for ip in LEAF_IP_ADDRESSES {
        san.ip(ip);
    }
    let san = san.build(&leaf.x509v3_context(Some(ca), None))?;
    leaf.append_extension(&san)?;
    let usage = ExtendedKeyUsage::new().server_auth().build()?;
    leaf.append_extension(&usage)?;
    leaf.sign(ca_key, MessageDigest::sha256())?;
    Ok(leaf.build())
}

fn acceptor(leaf_key: &PKey<Private>, leaf: &X509, ca: X509) -> Result<SslAcceptor, ErrorStack> {
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
    builder.set_private_key(leaf_key)?;
    builder.set_certificate(leaf)?;
    builder.add_extra_chain_cert(ca)?;
    builder.set_alpn_select_callback(|_, client| {
        select_next_proto(crate::tls::alpn::HTTP11_WIRE, client).ok_or(AlpnError::NOACK)
    });
    Ok(builder.build())
}

fn new_key() -> Result<PKey<Private>, ErrorStack> {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
    PKey::from_ec_key(EcKey::generate(&group)?)
}

fn name(common_name: &str) -> Result<X509Name, ErrorStack> {
    let mut builder = X509NameBuilder::new()?;
    builder.append_entry_by_text("CN", common_name)?;
    Ok(builder.build())
}

fn base_cert(
    subject: &X509Name,
    issuer: &leyline_bssl::x509::X509NameRef,
    key: &PKey<Private>,
) -> Result<X509Builder, ErrorStack> {
    let mut cert = X509::builder()?;
    cert.set_version(2)?;
    let mut serial = BigNum::new()?;
    serial.rand(SERIAL_BITS, MsbOption::MAYBE_ZERO, false)?;
    let serial = serial.to_asn1_integer()?;
    cert.set_serial_number(&serial)?;
    cert.set_subject_name(subject)?;
    cert.set_issuer_name(issuer)?;
    cert.set_pubkey(key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(VALID_DAYS)?;
    cert.set_not_before(&not_before)?;
    cert.set_not_after(&not_after)?;
    Ok(cert)
}
