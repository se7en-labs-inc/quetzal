//! Fresh private certificate fixtures shared by transport and process tests.
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

fn openssl(root: &Path, args: &[&str]) {
    let status = Command::new("openssl")
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("Nix test environment must supply openssl");
    assert!(status.success(), "openssl fixture generation failed");
}

pub fn generate(root: &Path) {
    for ca in ["ca", "foreign"] {
        openssl(
            root,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-days",
                "2",
                "-subj",
                &format!("/CN={ca}"),
                "-keyout",
                &format!("{ca}.key"),
                "-out",
                &format!("{ca}.crt"),
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
            ],
        );
    }
    for (leaf, ca, purpose) in [
        ("server", "ca", "serverAuth"),
        ("client", "ca", "clientAuth"),
        ("untrusted", "foreign", "clientAuth"),
    ] {
        openssl(
            root,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-subj",
                &format!("/CN={leaf}"),
                "-keyout",
                &format!("{leaf}.key"),
                "-out",
                &format!("{leaf}.csr"),
            ],
        );
        fs::write(root.join("extensions"), format!(
            "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage={purpose}\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n"
        )).unwrap();
        openssl(
            root,
            &[
                "x509",
                "-req",
                "-in",
                &format!("{leaf}.csr"),
                "-CA",
                &format!("{ca}.crt"),
                "-CAkey",
                &format!("{ca}.key"),
                "-CAcreateserial",
                "-days",
                "2",
                "-sha256",
                "-extfile",
                "extensions",
                "-out",
                &format!("{leaf}.crt"),
            ],
        );
    }
}
