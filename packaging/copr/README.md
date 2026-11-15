# Fedora COPR

`navia.spec` builds Navia from the release tag's source in [COPR](https://copr.fedorainfracloud.org/coprs/davekai/navia/),
with Fedora's own Rust, for x86_64 and aarch64. The project has network enabled in its builds, because Cargo
downloads the crates and `vendor/gpui/fetch-gpui.sh` downloads GPUI during the build.

Users install it with:

    sudo dnf copr enable davekai/navia
    sudo dnf install navia

## A new release

1. Tag the release `v<version>` on GitHub, as usual.
2. In `navia.spec`, set `Version` to the new version, set `Release` back to `1`, and add a `%changelog` entry.
3. Build the source RPM and send it to COPR (needs `rpmdevtools` and `copr-cli` with a token in `~/.config/copr`):

       spectool -g -C ~/rpmbuild/SOURCES packaging/copr/navia.spec
       rpmbuild -bs --define "_sourcedir $HOME/rpmbuild/SOURCES" --define "_srcrpmdir $PWD" packaging/copr/navia.spec
       copr-cli build davekai/navia navia-<version>-1.*.src.rpm

To change only the packaging, raise `Release` instead of `Version`.

Builds need Fedora's `rust` to be at least the version the code needs (1.99). Rawhide and current Fedora releases
update it.
