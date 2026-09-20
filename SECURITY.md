# Security

Do not include bearer tokens, private keys, model credentials, raw runtime
databases or unredacted appliance logs in public issues. Report vulnerabilities
through the repository's private vulnerability reporting channel when enabled;
otherwise contact the organization maintainers privately before disclosure.

Releases are signed with a dedicated Sparkplane minisign key. Installers pin the
public key and exact artifact digest; successful TLS download alone is not
release authorization. A signing key must never be committed or placed in a
build artifact. The `release` GitHub environment must require maintainer review
before its signing secrets become available.

The migration trust transition is signed by the already-installed sy release
authority and binds the new authority to an exact release, host identity and
expiration. Migration must not trust a replacement key merely because it was
downloaded alongside the new executable.

Pull-request CI runs on hosted CPU runners. GPU qualification is an operator
action using the existing finite, signed operation interface; untrusted PR code
must never run with appliance credentials or Docker privileges.

The root executor has no arbitrary shell/argv execution API. Keep peer UID
authorization, request bounds, content-addressed manifests, exact-container
cleanup, resource admission, AppArmor confinement and private engine networking
intact when adding workloads.
