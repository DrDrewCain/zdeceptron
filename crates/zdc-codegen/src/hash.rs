//! Content hashes, and the two properties a cache-busting name needs.
//!
//! A file whose URL never changes cannot be cached for long, because a
//! redeploy has no way to tell the browser that the bytes behind that URL
//! are different ones. Putting the hash of the content into the name makes
//! the URL a function of the bytes, and that is the whole mechanism: a new
//! version is a new URL, an old version is still at the old URL, and the
//! cache entry for either is correct forever. That is what earns
//! `immutable` in [`crate::cache`], and nothing else does.
//!
//! Two properties, and both directions matter:
//!
//! 1. **Identical content must produce an identical name.** Otherwise every
//!    build invalidates every cache, which is worse than not hashing at
//!    all — the visitor re-downloads a file that did not change *and* the
//!    build is no longer reproducible.
//! 2. **Different content must produce a different name.** Otherwise the
//!    browser serves the old bytes from a cache entry marked immutable, and
//!    there is no recovery short of a new file name: `immutable` means the
//!    browser is entitled not to ask.
//!
//! # Why FNV-1a, written here, and not a crate or `DefaultHasher`
//!
//! **Not [`std::collections::hash_map::DefaultHasher`].** It is documented
//! as not guaranteed to produce the same output across Rust releases, and
//! this hash lands in a file name that a deployment's cache keys are built
//! from. A compiler upgrade would silently rename every stylesheet in every
//! bundle — property 1 broken by the standard library, on a schedule nobody
//! controls.
//!
//! **Not a hashing crate.** `fnv` is already in `Cargo.lock`, but it enters
//! through `logos-codegen`, which is a proc-macro's own build dependency
//! and reaches no published artifact. Naming it here would add a real
//! dependency edge to a crate that is published, for the twelve lines
//! below.
//!
//! **Not SHA-256.** This hash is a cache key, not an integrity claim.
//! Nothing here defends against an adversary who can already write the file
//! being hashed, and the moment something *does* need that — a Subresource
//! Integrity attribute is the obvious candidate — it must use a
//! cryptographic hash chosen for that job rather than reuse this one. A
//! hash picked for cache keys is not a hash picked for integrity, and the
//! distinction is worth a sentence here so nobody has to rediscover it.
//!
//! FNV-1a is specified once and does not move: the offset basis and the
//! prime below are the published 64-bit parameters, and the same bytes
//! produce the same digest on every machine and every toolchain, which is
//! exactly property 1.

/// The 64-bit FNV-1a offset basis.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The 64-bit FNV-1a prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// The content hash of some bytes, as sixteen lowercase hexadecimal
/// characters.
///
/// Not truncated. A shorter name would be prettier and the collision that
/// a truncation permits is not a wrong render — it is an `immutable` cache
/// entry that the browser is entitled never to revisit, which is the one
/// failure in this crate with no recovery from the server side. Sixteen
/// characters cost sixteen bytes in a file name and buy the full width of
/// the digest.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// `relative`, with the content hash of `bytes` inserted before its
/// extension: `assets/site.css` becomes `assets/site.<hash>.css`.
///
/// The hash goes *before* the extension rather than after the whole name
/// because the extension is what a static host reads to decide the
/// `Content-Type`. A stylesheet served as `text/plain` is not applied, and
/// the console says nothing a reader would connect to a build step.
///
/// The directory is untouched, which is the property the asset directory
/// leans on: a relative `url(./Inter.woff2)` inside a hashed stylesheet
/// still resolves, because the stylesheet did not move — only its name
/// changed, and a relative URL is resolved against the directory.
pub fn hashed_name(relative: &str, bytes: &[u8]) -> String {
    let hash = content_hash(bytes);
    // The extension is the last dot in the *file name*, not in the path: a
    // directory called `v1.2` must not be mistaken for one.
    let start = match relative.rfind('/') {
        Some(slash) => slash + 1,
        None => 0,
    };
    match relative[start..].rfind('.') {
        // A leading dot is not an extension, it is a hidden file — and the
        // asset walk drops those before they reach here, so this arm is
        // about not being surprising rather than about a case that occurs.
        Some(dot) if dot > 0 => {
            let split = start + dot;
            format!("{}.{hash}{}", &relative[..split], &relative[split..])
        }
        _ => format!("{relative}.{hash}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Property 1, in the only form that matters: two builds of an
    /// unchanged file agree.
    #[test]
    fn identical_content_hashes_the_same_every_time() {
        let bytes = b"body { color: rebeccapurple; }";
        assert_eq!(content_hash(bytes), content_hash(bytes));
        assert_eq!(
            hashed_name("assets/site.css", bytes),
            hashed_name("assets/site.css", bytes)
        );
    }

    /// Property 2. One byte, and it must be a different URL — an
    /// `immutable` entry under the old one is never revisited.
    #[test]
    fn one_changed_byte_changes_the_name() {
        let before = hashed_name("assets/site.css", b"body { color: red; }");
        let after = hashed_name("assets/site.css", b"body { color: rex; }");
        assert_ne!(before, after);
    }

    /// The published FNV-1a 64 vector, so a future rewrite of the loop
    /// cannot quietly change every file name in every bundle.
    #[test]
    fn the_digest_is_the_published_fnv_1a_64_of_the_input() {
        assert_eq!(content_hash(b""), "cbf29ce484222325");
        assert_eq!(content_hash(b"a"), "af63dc4c8601ec8c");
        assert_eq!(content_hash(b"foobar"), "85944171f73967e8");
    }

    #[test]
    fn the_hash_goes_before_the_extension_and_the_directory_is_untouched() {
        let name = hashed_name("assets/deep/site.css", b"a{}");
        assert!(name.starts_with("assets/deep/site."), "{name}");
        assert!(name.ends_with(".css"), "{name}");
        assert_eq!(name.matches('/').count(), 2);
    }

    /// `site.min.css` keeps `.min` as part of its name: only the last dot
    /// is the extension.
    #[test]
    fn only_the_last_dot_is_the_extension() {
        let name = hashed_name("assets/site.min.css", b"a{}");
        assert!(name.starts_with("assets/site.min."), "{name}");
        assert!(name.ends_with(".css"), "{name}");
    }

    /// A dot in a directory is not an extension.
    #[test]
    fn a_dotted_directory_is_not_mistaken_for_an_extension() {
        let name = hashed_name("assets/v1.2/LICENSE", b"a{}");
        assert!(name.starts_with("assets/v1.2/LICENSE."), "{name}");
    }

    #[test]
    fn a_name_with_no_extension_takes_the_hash_at_the_end() {
        let name = hashed_name("assets/LICENSE", b"a{}");
        assert_eq!(name, format!("assets/LICENSE.{}", content_hash(b"a{}")));
    }
}
