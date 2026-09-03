#!/bin/sh
set -eu

restty_repo=${RESTTY_REPO:-restty-cli/restty}
restty_install_dir=${RESTTY_INSTALL_DIR:-"${HOME}/.local/bin"}
restty_version=${RESTTY_VERSION:-latest}

restty_os=$(uname -s)
restty_arch=$(uname -m)

case "${restty_arch}" in
    x86_64|amd64) restty_arch=x86_64 ;;
    aarch64|arm64) restty_arch=aarch64 ;;
    *) printf '%s\n' "restty: unsupported architecture: ${restty_arch}" >&2; exit 1 ;;
esac

case "${restty_os}" in
    Linux) restty_target="${restty_arch}-unknown-linux-musl" ;;
    Darwin) restty_target="${restty_arch}-apple-darwin" ;;
    FreeBSD) restty_target="${restty_arch}-unknown-freebsd" ;;
    OpenBSD) restty_target="${restty_arch}-unknown-openbsd" ;;
    NetBSD) restty_target="${restty_arch}-unknown-netbsd" ;;
    *) printf '%s\n' "restty: unsupported operating system: ${restty_os}" >&2; exit 1 ;;
esac

restty_asset="restty-${restty_target}.tar.gz"
if [ "${restty_version}" = latest ]; then
    restty_base_url="https://github.com/${restty_repo}/releases/latest/download"
else
    restty_base_url="https://github.com/${restty_repo}/releases/download/${restty_version}"
fi

restty_tmp_dir=$(mktemp -d 2>/dev/null || mktemp -d -t restty)
trap 'rm -rf "${restty_tmp_dir}"' EXIT HUP INT TERM

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v fetch >/dev/null 2>&1; then
        fetch -q -o "$2" "$1"
    else
        printf '%s\n' 'restty: curl or fetch is required for installation' >&2
        exit 1
    fi
}

download "${restty_base_url}/${restty_asset}" "${restty_tmp_dir}/${restty_asset}"
download "${restty_base_url}/checksums.txt" "${restty_tmp_dir}/checksums.txt"

restty_expected=$(awk -v asset="${restty_asset}" '$2 == asset || $2 == "*" asset { print $1; exit }' "${restty_tmp_dir}/checksums.txt")
if [ -z "${restty_expected}" ]; then
    printf '%s\n' "restty: no checksum published for ${restty_asset}" >&2
    exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
    restty_actual=$(sha256sum "${restty_tmp_dir}/${restty_asset}" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
    restty_actual=$(shasum -a 256 "${restty_tmp_dir}/${restty_asset}" | awk '{print $1}')
elif command -v sha256 >/dev/null 2>&1; then
    restty_actual=$(sha256 -q "${restty_tmp_dir}/${restty_asset}")
else
    printf '%s\n' 'restty: no SHA-256 utility found' >&2
    exit 1
fi

if [ "${restty_actual}" != "${restty_expected}" ]; then
    printf '%s\n' "restty: checksum verification failed for ${restty_asset}" >&2
    exit 1
fi

tar -xzf "${restty_tmp_dir}/${restty_asset}" -C "${restty_tmp_dir}"
mkdir -p "${restty_install_dir}"
install -m 0755 "${restty_tmp_dir}/restty" "${restty_install_dir}/restty"
printf '%s\n' "restty installed to ${restty_install_dir}/restty"
printf '%s\n' 'Add eval "$(restty init bash)" or eval "$(restty init zsh)" to your shell startup file.'
