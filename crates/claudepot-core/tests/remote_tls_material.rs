//! Does `scripts/mint-remote-cert.sh`'s real output load into rustls?
//!
//! The unit tests in `remote::serve` use synthetic PEM, which proves the
//! error paths and nothing about the certificate the user is actually
//! told to mint. `#[ignore]` because it needs the script to have been
//! run into `CLAUDEPOT_DATA_DIR` first:
//!
//! ```bash
//! CLAUDEPOT_DATA_DIR=/tmp/x ./scripts/mint-remote-cert.sh host.local
//! CLAUDEPOT_DATA_DIR=/tmp/x cargo test -p claudepot-core \
//!   --test remote_tls_material -- --ignored --nocapture
//! ```

use claudepot_core::remote::{serve, tls};

#[test]
#[ignore = "needs scripts/mint-remote-cert.sh run into CLAUDEPOT_DATA_DIR"]
fn the_minted_certificate_is_usable_by_rustls() {
    let material = tls::load().expect("load TLS material");
    println!("cert: {}", material.cert_path.display());
    println!("key : {}", material.key_path.display());

    let config = serve::tls_config(&material).expect("rustls accepted the material");
    assert_eq!(
        config.alpn_protocols,
        vec![b"http/1.1".to_vec()],
        "the connection handler speaks HTTP/1.1; advertising h2 would \
         negotiate a protocol it does not implement"
    );
    println!("rustls accepted the minted certificate");
}
