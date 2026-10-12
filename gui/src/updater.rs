//! Updating the installed binary in place.
//!
//! Tauri's own updater can only replace an AppImage on Linux, and an AppImage
//! is 79 MB against a few MB for the plain binary, so this app updates itself
//! with a small mechanism of its own. It leans on [Minisign], the signature
//! scheme Tauri's updater uses too, and on nothing else that needs trusting
//! beyond the one public key compiled in below.
//!
//! A release publishes, next to the binary, a `latest.json`:
//!
//! ```json
//! { "version": "0.2.0", "url": "https://…/nexus-menu-gui-0.2.0-x86_64-linux",
//!   "signature": "<the .minisig file's text>", "notes": "…" }
//! ```
//!
//! and signs the binary with the trusted comment `nexus-menu-gui version=0.2.0`.
//! The comment is covered by the signature, which is what stops a rollback: an
//! attacker who can serve files cannot pass an old, validly signed binary off
//! as a new version, because the version the signature vouches for has to be
//! the version the manifest claims and has to be newer than the one running.
//!
//! Nothing is fetched except over https, nothing is trusted until its
//! signature checks out, and the running binary is replaced with a rename so a
//! failure at any point leaves the installed one untouched.
//!
//! [Minisign]: https://jedisct1.github.io/minisign/

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};

/// Where the newest release publishes its manifest.
pub const MANIFEST_URL: &str =
    "https://github.com/wlfogle/nexus-menu/releases/latest/download/latest.json";

/// Where a person can fetch a release by hand when this one cannot update
/// itself.
pub const RELEASES_URL: &str = "https://github.com/wlfogle/nexus-menu/releases/latest";

/// The key every release is signed with. The secret half never leaves the
/// maintainer's machine and the release build's secrets.
pub const PUBLIC_KEY: &str = "RWSCvzzDwUGzcj+r+gMxp0xOOsnu3D/HmB0vq5E1nGVqOqGl5+b6ymS2";

/// What the signed trusted comment says before the version number.
const SIGNED_COMMENT_PREFIX: &str = "nexus-menu-gui version=";

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_BINARY_BYTES: u64 = 128 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

/// Gets a URL's body. A trait so the logic can be tested without a network.
pub trait Fetch {
    /// The body of `url`, refusing anything over `max_bytes`.
    fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, String>;
}

/// The real thing, over https only.
pub struct Http {
    allow_insecure: bool,
}

impl Http {
    /// https only, including across redirects.
    pub fn secure() -> Self {
        Http {
            allow_insecure: false,
        }
    }

    /// Plain http as well, so a test can talk to a server on this machine.
    /// Never used by a release build.
    #[cfg(any(test, feature = "update-test-hooks"))]
    pub fn insecure() -> Self {
        Http {
            allow_insecure: true,
        }
    }
}

impl Fetch for Http {
    fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
        if !self.allow_insecure && !url.starts_with("https://") {
            return Err(format!(
                "refusing {url}: updates are only fetched over https"
            ));
        }
        // A manifest is tiny; a binary on a slow link is not.
        let timeout = if max_bytes <= MAX_MANIFEST_BYTES {
            Duration::from_secs(20)
        } else {
            Duration::from_secs(300)
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .https_only(!self.allow_insecure)
            .build()
            .into();
        let mut response = agent
            .get(url)
            .call()
            .map_err(|e| format!("could not fetch {url}: {e}"))?;
        response
            .body_mut()
            .with_config()
            .limit(max_bytes)
            .read_to_vec()
            .map_err(|e| match e {
                ureq::Error::BodyExceedsLimit(limit) => format!(
                    "{url} is larger than the {} the app will download, so it was refused",
                    human_size(limit)
                ),
                other => format!("could not read {url}: {other}"),
            })
    }
}

/// `bytes` as KiB or MiB, for a message a person reads.
fn human_size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

// ---------------------------------------------------------------------------
// Checking and verifying
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    url: String,
    signature: String,
    #[serde(default)]
    notes: String,
}

/// A newer release that has been offered, and not yet trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    url: String,
    signature: String,
}

fn parse_version(text: &str) -> Result<Version, String> {
    Version::parse(text.trim()).map_err(|e| format!("`{text}` is not a version number: {e}"))
}

/// Check that `signature` is `public_key`'s signature of `data`, and that what
/// it signed was release `expected`.
fn verify(
    public_key: &str,
    data: &[u8],
    signature: &str,
    expected: &Version,
) -> Result<(), String> {
    let key = PublicKey::from_base64(public_key)
        .map_err(|e| format!("the built-in public key is unusable: {e}"))?;
    let signature =
        Signature::decode(signature).map_err(|e| format!("the signature is malformed: {e}"))?;
    key.verify(data, &signature, false)
        .map_err(|e| format!("the signature does not match the download: {e}"))?;

    let comment = signature.trusted_comment();
    let signed = comment
        .strip_prefix(SIGNED_COMMENT_PREFIX)
        .ok_or("the signature does not say which version it signs")?;
    let signed = parse_version(signed)?;
    if &signed != expected {
        return Err(format!(
            "the download is signed as version {signed}, not {expected}"
        ));
    }
    Ok(())
}

pub struct Updater<'a, F: Fetch> {
    pub fetcher: &'a F,
    pub manifest_url: &'a str,
    pub public_key: &'a str,
    /// The version that is running.
    pub current: &'a str,
}

impl<F: Fetch> Updater<'_, F> {
    /// The release on offer, if it is newer than the one running.
    pub fn check(&self) -> Result<Option<Update>, String> {
        let current = parse_version(self.current)?;
        let body = self.fetcher.get(self.manifest_url, MAX_MANIFEST_BYTES)?;
        let manifest: Manifest = serde_json::from_slice(&body)
            .map_err(|e| format!("the update manifest is not valid: {e}"))?;
        let offered = parse_version(&manifest.version)?;
        if offered <= current {
            return Ok(None);
        }
        Ok(Some(Update {
            version: offered.to_string(),
            notes: manifest.notes,
            url: manifest.url,
            signature: manifest.signature,
        }))
    }

    /// Download `update` and return it only if it is genuine: signed by our
    /// key, signed as exactly this version, newer than the one running, and an
    /// executable.
    pub fn fetch_verified(&self, update: &Update) -> Result<Vec<u8>, String> {
        let current = parse_version(self.current)?;
        let offered = parse_version(&update.version)?;
        if offered <= current {
            return Err(format!("{offered} is not newer than {current}"));
        }
        let binary = self.fetcher.get(&update.url, MAX_BINARY_BYTES)?;
        verify(self.public_key, &binary, &update.signature, &offered)?;
        if !binary.starts_with(b"\x7fELF") {
            return Err("the download is signed but is not an executable".to_string());
        }
        Ok(binary)
    }
}

// ---------------------------------------------------------------------------
// Replacing and restarting
// ---------------------------------------------------------------------------

/// Whether the binary at `exe` is somewhere an update should replace it. A
/// build in a cargo `target` directory is being developed, not installed.
pub fn install_location(exe: &Path) -> Result<(), String> {
    if exe.components().any(|c| c.as_os_str() == "target") {
        return Err(format!(
            "{} is a build in progress, not an installed copy; it does not update itself",
            exe.display()
        ));
    }
    Ok(())
}

/// Put `bytes` where `exe` is, atomically.
///
/// The new file is written beside the old one (so the rename stays on one
/// filesystem), made executable, flushed, and then renamed over it. A rename
/// over a running executable is safe on Linux: the running process keeps its
/// old copy until it exits. If anything fails the old binary is untouched and
/// the half-written file is removed.
pub fn replace_binary(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no directory", exe.display()))?;
    let name = exe
        .file_name()
        .ok_or_else(|| format!("{} has no file name", exe.display()))?
        .to_string_lossy();
    let staging = dir.join(format!(".{name}.update-{}", std::process::id()));

    let written = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&staging)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, exe)
    })();

    if let Err(e) = written {
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "could not replace {}: {e}. Download the new version by hand from {RELEASES_URL}",
            exe.display()
        ));
    }
    Ok(())
}

/// Start `exe` again with the arguments the app was started with, in its own
/// process group so it outlives this process.
pub fn restart(exe: &Path, args: &[String]) -> Result<(), String> {
    Command::new(exe)
        .args(args)
        .process_group(0)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start {}: {e}", exe.display()))
}

// ---------------------------------------------------------------------------
// What the window sees
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Available {
    pub version: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateView {
    /// The version that is running.
    pub current: String,
    pub available: Option<Available>,
    /// Whether this copy can replace itself. When it cannot, `problem` says why
    /// and the page points at the releases page instead.
    pub can_install: bool,
    pub problem: Option<String>,
    pub releases_url: &'static str,
}

/// The window's view of updating. The page never supplies a URL or a path: it
/// asks whether there is an update and then to install the one that was found.
pub struct UpdateService<F: Fetch> {
    fetcher: F,
    manifest_url: String,
    public_key: String,
    current: String,
    enabled: bool,
    found: Mutex<Option<Update>>,
}

impl UpdateService<Http> {
    /// The shipped configuration: our manifest, our key, https only.
    ///
    /// With the `update-test-hooks` feature (never on in a release) the
    /// manifest address and key can be replaced from the environment, so the
    /// whole update can be rehearsed against a server on this machine.
    pub fn shipped(enabled: bool) -> Self {
        #[cfg(feature = "update-test-hooks")]
        if let (Ok(url), Ok(key)) = (
            std::env::var("NEXUS_MENU_TEST_UPDATE_URL"),
            std::env::var("NEXUS_MENU_TEST_UPDATE_KEY"),
        ) {
            return UpdateService::new(Http::insecure(), url, key, enabled);
        }
        UpdateService::new(
            Http::secure(),
            MANIFEST_URL.to_string(),
            PUBLIC_KEY.to_string(),
            enabled,
        )
    }
}

impl<F: Fetch> UpdateService<F> {
    pub fn new(fetcher: F, manifest_url: String, public_key: String, enabled: bool) -> Self {
        UpdateService {
            fetcher,
            manifest_url,
            public_key,
            current: env!("CARGO_PKG_VERSION").to_string(),
            enabled,
            found: Mutex::new(None),
        }
    }

    fn updater(&self) -> Updater<'_, F> {
        Updater {
            fetcher: &self.fetcher,
            manifest_url: &self.manifest_url,
            public_key: &self.public_key,
            current: &self.current,
        }
    }

    fn found(&self) -> std::sync::MutexGuard<'_, Option<Update>> {
        self.found.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask whether a newer release exists. Makes no request when updating is
    /// switched off.
    pub fn check(&self) -> Result<UpdateView, String> {
        let mut view = UpdateView {
            current: self.current.clone(),
            available: None,
            can_install: false,
            problem: None,
            releases_url: RELEASES_URL,
        };
        if !self.enabled {
            return Ok(view);
        }

        let update = self.updater().check()?;
        if let Some(update) = &update {
            view.available = Some(Available {
                version: update.version.clone(),
                notes: update.notes.clone(),
            });
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            match install_location(&exe) {
                Ok(()) => view.can_install = true,
                Err(problem) => view.problem = Some(problem),
            }
        }
        *self.found() = update;
        Ok(view)
    }

    /// Download, verify and install the release [`UpdateService::check`] found,
    /// over the binary at `exe`.
    pub fn install_to(&self, exe: &Path) -> Result<String, String> {
        let update = self
            .found()
            .clone()
            .ok_or("no update has been found; check for one first")?;
        install_location(exe)?;
        let binary = self.updater().fetch_verified(&update)?;
        replace_binary(exe, &binary)?;
        Ok(update.version)
    }

    /// [`UpdateService::install_to`] for the running binary. Returns where it
    /// is, taken before the replacement: afterwards the kernel reports the old
    /// file as deleted.
    pub fn install(&self) -> Result<(PathBuf, String), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let version = self.install_to(&exe)?;
        Ok((exe, version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;

    // --- throwaway keys and signatures, made in-process -------------------

    struct Keys {
        public: String,
        secret: minisign::SecretKey,
    }

    fn keys() -> Keys {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        Keys {
            public: pair.pk.to_base64(),
            secret: pair.sk,
        }
    }

    fn sign_with_comment(keys: &Keys, data: &[u8], comment: &str) -> String {
        minisign::sign(
            None,
            &keys.secret,
            std::io::Cursor::new(data),
            Some(comment),
            None,
        )
        .unwrap()
        .into_string()
    }

    fn sign(keys: &Keys, data: &[u8], version: &str) -> String {
        sign_with_comment(keys, data, &format!("{SIGNED_COMMENT_PREFIX}{version}"))
    }

    fn elf(extra: &str) -> Vec<u8> {
        let mut bytes = b"\x7fELF".to_vec();
        bytes.extend_from_slice(extra.as_bytes());
        bytes
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    // --- an in-memory network ---------------------------------------------

    struct Fake(HashMap<String, Vec<u8>>);

    impl Fetch for Fake {
        fn get(&self, url: &str, _max: u64) -> Result<Vec<u8>, String> {
            self.0.get(url).cloned().ok_or(format!("404 {url}"))
        }
    }

    fn manifest(version: &str, url: &str, signature: &str) -> Vec<u8> {
        serde_json::json!({
            "version": version, "url": url, "signature": signature, "notes": "fixes"
        })
        .to_string()
        .into_bytes()
    }

    /// A release at `version` of `binary`, signed by `keys`, in a fake network.
    fn release(keys: &Keys, version: &str, binary: &[u8]) -> Fake {
        let url = format!("https://example.test/gui-{version}");
        let mut net = HashMap::new();
        net.insert(
            "https://example.test/latest.json".to_string(),
            manifest(version, &url, &sign(keys, binary, version)),
        );
        net.insert(url, binary.to_vec());
        Fake(net)
    }

    fn updater<'a>(net: &'a Fake, keys: &'a Keys, current: &'a str) -> Updater<'a, Fake> {
        Updater {
            fetcher: net,
            manifest_url: "https://example.test/latest.json",
            public_key: &keys.public,
            current,
        }
    }

    // --- signatures -----------------------------------------------------------

    #[test]
    fn a_good_signature_for_the_expected_version_verifies() {
        let k = keys();
        let data = elf("payload");
        verify(&k.public, &data, &sign(&k, &data, "0.2.0"), &v("0.2.0")).unwrap();
    }

    #[test]
    fn a_tampered_binary_is_rejected() {
        let k = keys();
        let data = elf("payload");
        let sig = sign(&k, &data, "0.2.0");
        let mut bad = data.clone();
        *bad.last_mut().unwrap() ^= 1;
        let err = verify(&k.public, &bad, &sig, &v("0.2.0")).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn a_signature_from_another_key_is_rejected() {
        let ours = keys();
        let theirs = keys();
        let data = elf("payload");
        let err = verify(
            &ours.public,
            &data,
            &sign(&theirs, &data, "0.2.0"),
            &v("0.2.0"),
        )
        .unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn the_signed_version_cannot_be_edited_after_the_fact() {
        // The anti-rollback guarantee rests on the trusted comment being
        // covered by the signature. Prove it by editing one.
        let k = keys();
        let data = elf("payload");
        let sig = sign(&k, &data, "0.2.0");
        assert!(sig.contains("version=0.2.0"));
        let forged = sig.replace("version=0.2.0", "version=9.9.9");
        let err = verify(&k.public, &data, &forged, &v("9.9.9")).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn an_old_release_cannot_pass_as_a_new_version() {
        let k = keys();
        let data = elf("old build");
        // Validly signed, but as 0.1.5; the manifest claims 0.3.0.
        let err = verify(&k.public, &data, &sign(&k, &data, "0.1.5"), &v("0.3.0")).unwrap_err();
        assert!(err.contains("signed as version 0.1.5, not 0.3.0"), "{err}");
    }

    #[test]
    fn a_signature_that_names_no_version_is_rejected() {
        let k = keys();
        let data = elf("payload");
        let sig = sign_with_comment(&k, &data, "timestamp:1 file:gui");
        let err = verify(&k.public, &data, &sig, &v("0.2.0")).unwrap_err();
        assert!(err.contains("does not say which version"), "{err}");
    }

    #[test]
    fn garbage_signature_and_garbage_key_are_errors_not_panics() {
        let k = keys();
        let data = elf("x");
        assert!(verify(&k.public, &data, "not a signature", &v("1.0.0")).is_err());
        assert!(verify("not a key", &data, &sign(&k, &data, "1.0.0"), &v("1.0.0")).is_err());
    }

    #[test]
    fn the_shipped_public_key_parses() {
        PublicKey::from_base64(PUBLIC_KEY).expect("the compiled-in key must be valid");
    }

    // --- checking -------------------------------------------------------------

    #[test]
    fn a_newer_release_is_offered() {
        let k = keys();
        let net = release(&k, "0.2.0", &elf("new"));
        let found = updater(&net, &k, "0.1.0").check().unwrap().unwrap();
        assert_eq!(found.version, "0.2.0");
        assert_eq!(found.notes, "fixes");
    }

    #[test]
    fn the_same_or_an_older_release_is_not_offered() {
        let k = keys();
        let net = release(&k, "0.2.0", &elf("new"));
        assert_eq!(updater(&net, &k, "0.2.0").check().unwrap(), None);
        assert_eq!(updater(&net, &k, "0.9.0").check().unwrap(), None);
    }

    #[test]
    fn versions_compare_as_versions_not_as_text() {
        let k = keys();
        let net = release(&k, "0.10.0", &elf("new"));
        // As text "0.10.0" < "0.9.0"; as versions it is newer.
        assert!(updater(&net, &k, "0.9.0").check().unwrap().is_some());
    }

    #[test]
    fn a_broken_manifest_or_version_is_an_error() {
        let k = keys();
        let mut net = HashMap::new();
        net.insert(
            "https://example.test/latest.json".to_string(),
            b"{nope".to_vec(),
        );
        let net = Fake(net);
        assert!(updater(&net, &k, "0.1.0").check().is_err());

        let mut net = HashMap::new();
        net.insert(
            "https://example.test/latest.json".to_string(),
            manifest("not-a-version", "https://x", "sig"),
        );
        let net = Fake(net);
        let err = updater(&net, &k, "0.1.0").check().unwrap_err();
        assert!(err.contains("not a version number"), "{err}");

        let empty = Fake(HashMap::new());
        assert!(updater(&empty, &k, "0.1.0").check().is_err());
    }

    // --- fetching and verifying a release -----------------------------------

    #[test]
    fn a_genuine_release_is_downloaded() {
        let k = keys();
        let binary = elf("the new build");
        let net = release(&k, "0.2.0", &binary);
        let u = updater(&net, &k, "0.1.0");
        let update = u.check().unwrap().unwrap();
        assert_eq!(u.fetch_verified(&update).unwrap(), binary);
    }

    #[test]
    fn a_swapped_binary_is_rejected() {
        let k = keys();
        let net = release(&k, "0.2.0", &elf("the real build"));
        let u = updater(&net, &k, "0.1.0");
        let update = u.check().unwrap().unwrap();

        // The signature is genuine but the file behind the URL is not what was
        // signed.
        let mut tampered = net.0.clone();
        tampered.insert(update.url.clone(), elf("malware"));
        let net2 = Fake(tampered);
        let err = updater(&net2, &k, "0.1.0")
            .fetch_verified(&update)
            .unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn a_signed_file_that_is_not_an_executable_is_rejected() {
        let k = keys();
        let net = release(&k, "0.2.0", b"<html>an error page</html>");
        let u = updater(&net, &k, "0.1.0");
        let update = u.check().unwrap().unwrap();
        let err = u.fetch_verified(&update).unwrap_err();
        assert!(err.contains("not an executable"), "{err}");
    }

    #[test]
    fn a_rollback_through_the_manifest_is_rejected() {
        let k = keys();
        let old = elf("0.1.5 build");
        let url = "https://example.test/gui";
        let mut net = HashMap::new();
        // The manifest claims 0.3.0 and carries 0.1.5's genuine signature.
        net.insert(
            "https://example.test/latest.json".to_string(),
            manifest("0.3.0", url, &sign(&k, &old, "0.1.5")),
        );
        net.insert(url.to_string(), old);
        let net = Fake(net);
        let u = updater(&net, &k, "0.2.0");
        let update = u.check().unwrap().unwrap();
        let err = u.fetch_verified(&update).unwrap_err();
        assert!(err.contains("signed as version 0.1.5"), "{err}");
    }

    #[test]
    fn an_update_that_is_not_newer_is_never_downloaded() {
        let k = keys();
        let net = release(&k, "0.2.0", &elf("new"));
        let u = updater(&net, &k, "0.1.0");
        let update = u.check().unwrap().unwrap();
        // The running version moved on between the check and the install.
        let err = updater(&net, &k, "0.2.0")
            .fetch_verified(&update)
            .unwrap_err();
        assert!(err.contains("not newer"), "{err}");
    }

    // --- replacing the binary ---------------------------------------------------

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nexus-menu-upd-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn replacing_swaps_the_content_keeps_it_executable_and_leaves_no_litter() {
        let dir = scratch("replace");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();

        replace_binary(&exe, b"\x7fELFnew").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"\x7fELFnew");
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(names(&dir), vec!["app"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_executable_can_be_replaced_and_keeps_running() {
        // The claim the whole mechanism rests on: rename over a binary that is
        // executing. Copy a real executable, run it, replace it, and see that
        // the running process was not disturbed.
        let dir = scratch("running");
        let exe = dir.join("sleeper");
        fs::copy("/bin/sleep", &exe).unwrap();
        let mut child = Command::new(&exe).arg("30").spawn().unwrap();

        replace_binary(&exe, b"\x7fELFreplacement").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"\x7fELFreplacement");
        assert!(child.try_wait().unwrap().is_none(), "the process died");
        child.kill().unwrap();
        child.wait().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replacing_in_a_missing_directory_fails_cleanly() {
        let dir = scratch("missing");
        let exe = dir.join("no/such/dir/app");
        let err = replace_binary(&exe, b"x").unwrap_err();
        assert!(err.contains("could not replace"), "{err}");
        assert!(err.contains(RELEASES_URL), "{err}");
        assert_eq!(names(&dir), Vec::<String>::new());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replacing_in_a_read_only_directory_fails_and_the_old_binary_survives() {
        let dir = scratch("readonly");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        // Root ignores directory permissions, so there is nothing to test.
        let probe = dir.join("probe");
        if fs::write(&probe, b"").is_ok() {
            let _ = fs::remove_file(&probe);
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let err = replace_binary(&exe, b"new").unwrap_err();
        assert!(err.contains("could not replace"), "{err}");
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert_eq!(names(&dir), vec!["app"]);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cargo_build_directory_is_not_an_install_location() {
        assert!(install_location(Path::new("/home/u/.local/bin/nexus-menu-gui")).is_ok());
        assert!(install_location(Path::new("/usr/local/bin/nexus-menu-gui")).is_ok());
        let err = install_location(Path::new("/r/nexus-menu/gui/target/debug/nexus-menu-gui"))
            .unwrap_err();
        assert!(err.contains("build in progress"), "{err}");
    }

    // --- the service ----------------------------------------------------------

    fn service(net: Fake, k: &Keys, enabled: bool) -> UpdateService<Fake> {
        let mut s = UpdateService::new(
            net,
            "https://example.test/latest.json".to_string(),
            k.public.clone(),
            enabled,
        );
        s.current = "0.1.0".to_string();
        s
    }

    #[test]
    fn a_disabled_service_makes_no_request_and_offers_nothing() {
        let k = keys();
        // An empty network would make any request fail.
        let view = service(Fake(HashMap::new()), &k, false).check().unwrap();
        assert_eq!(view.available, None);
        assert!(!view.can_install);
        assert_eq!(view.current, "0.1.0");
    }

    #[test]
    fn installing_before_any_check_is_refused() {
        let k = keys();
        let s = service(release(&k, "0.2.0", &elf("n")), &k, true);
        let dir = scratch("nocheck");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        let err = s.install_to(&exe).unwrap_err();
        assert!(err.contains("check for one first"), "{err}");
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_then_install_replaces_the_binary_with_the_verified_release() {
        let k = keys();
        let new = elf("version two");
        let s = service(release(&k, "0.2.0", &new), &k, true);
        let view = s.check().unwrap();
        assert_eq!(
            view.available.as_ref().map(|a| a.version.as_str()),
            Some("0.2.0")
        );

        let dir = scratch("install");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        assert_eq!(s.install_to(&exe).unwrap(), "0.2.0");
        assert_eq!(fs::read(&exe).unwrap(), new);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_verification_leaves_the_installed_binary_untouched() {
        let k = keys();
        let other = keys();
        // Published under a key that is not ours.
        let s = service(release(&other, "0.2.0", &elf("evil")), &k, true);
        s.check().unwrap();

        let dir = scratch("untouched");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        let err = s.install_to(&exe).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        assert_eq!(names(&dir), vec!["app"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_directory_binary_is_never_replaced() {
        let k = keys();
        let s = service(release(&k, "0.2.0", &elf("n")), &k, true);
        s.check().unwrap();
        let dir = scratch("buildtree").join("target/debug");
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        let err = s.install_to(&exe).unwrap_err();
        assert!(err.contains("build in progress"), "{err}");
        assert_eq!(fs::read(&exe).unwrap(), b"old");
        let _ = fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }

    // --- the real HTTP client against a server on this machine ------------------

    /// Serve routes (path -> body) over plain http on a free local port and
    /// return its address. `routes` is given that address first, because a
    /// manifest has to name the server it lives on.
    fn serve(routes: impl FnOnce(&str) -> HashMap<String, Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = routes(&base);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                // Drain the headers.
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) <= 2 {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match routes.get(&path) {
                    Some(body) => ("200 OK", body.clone()),
                    None => ("404 Not Found", b"missing".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
                let mut sink = Vec::new();
                let _ = reader.read_to_end(&mut sink);
            }
        });
        base
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(1_000), "1 KiB");
        assert_eq!(human_size(128 * 1024 * 1024), "128 MiB");
        assert_eq!(human_size(134_217_728), "128 MiB");
    }

    #[test]
    fn the_secure_client_refuses_plain_http_without_touching_the_network() {
        // Port 1 would refuse a connection; the error must come first.
        let err = Http::secure()
            .get("http://127.0.0.1:1/latest.json", 100)
            .unwrap_err();
        assert!(err.contains("only fetched over https"), "{err}");
        let err = Http::secure().get("ftp://example.test/x", 100).unwrap_err();
        assert!(err.contains("only fetched over https"), "{err}");
    }

    #[test]
    fn the_client_fetches_refuses_oversize_and_reports_404() {
        let mut routes = HashMap::new();
        routes.insert("/small".to_string(), b"hello".to_vec());
        routes.insert("/big".to_string(), vec![7u8; 10_000]);
        let base = serve(|_| routes);
        let http = Http::insecure();

        assert_eq!(http.get(&format!("{base}/small"), 100).unwrap(), b"hello");
        let err = http.get(&format!("{base}/big"), 1_000).unwrap_err();
        assert!(err.contains("larger than the 1 KiB"), "{err}");
        assert!(err.contains("refused"), "{err}");
        let err = http.get(&format!("{base}/nope"), 100).unwrap_err();
        assert!(err.contains("could not fetch"), "{err}");
    }

    #[test]
    fn a_whole_update_over_real_http_is_found_verified_and_installed() {
        let k = keys();
        let new = elf("the build served over http");
        let signature = sign(&k, &new, "0.2.0");

        let base = serve(|base| {
            let mut routes = HashMap::new();
            routes.insert("/gui-0.2.0".to_string(), new.clone());
            routes.insert(
                "/latest.json".to_string(),
                manifest("0.2.0", &format!("{base}/gui-0.2.0"), &signature),
            );
            routes
        });

        let mut s = UpdateService::new(
            Http::insecure(),
            format!("{base}/latest.json"),
            k.public.clone(),
            true,
        );
        s.current = "0.1.0".to_string();
        let view = s.check().unwrap();
        assert_eq!(view.available.map(|a| a.version), Some("0.2.0".to_string()));

        let dir = scratch("e2e");
        let exe = dir.join("app");
        fs::write(&exe, b"old").unwrap();
        assert_eq!(s.install_to(&exe).unwrap(), "0.2.0");
        assert_eq!(fs::read(&exe).unwrap(), new);
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
