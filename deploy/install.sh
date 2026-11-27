#!/bin/sh
# One-command install for the robroz host (Docker, linux/amd64).
#
#   curl -fsSL https://raw.githubusercontent.com/PandaQuQ/robroz/main/deploy/install.sh | sh
#
# Pass options through the pipe with `sh -s --`:
#
#   curl -fsSL .../install.sh | sh -s -- --dry-run
#   curl -fsSL .../install.sh | ROBROZ_DIR=/opt/robroz sh
#
# What it does:
#   1. checks that docker and the `docker compose` plugin work;
#   2. creates the install directory (default ~/robroz);
#   3. writes compose.yml (always, the old copy is kept as compose.yml.bak when
#      it differs), and .env, secrets/robroz_token and data/ only if missing;
#   4. pulls the host and runner images (skipped when they are already local, or when --no-pull is set) and runs `docker compose up -d`;
#   5. prints the URL and the next steps.
#
# A rerun upgrades: it pulls the images for the version this script (or
# --version / ROBROZ_VERSION) names and recreates the container, unless the
# images are already local or --no-pull is set. It never overwrites .env,
# the token, or anything under data/. It also tightens the data directory
# to mode 0700 and the database to 0600.
#
# It asks for no credentials and writes none. The only secret it makes is the
# API token, generated locally and stored in secrets/robroz_token. After the
# host image is pulled, the file is owned by you, group is the container's
# gid, mode 0640, so both you and the host process can read it. The model
# account is entered later, in the client.
#
# Read docs/docker-host.md first: the host container gets docker.sock, which is
# root-equivalent on this machine.

set -eu

REPO="PandaQuQ/robroz"
HOST_IMAGE="ghcr.io/pandaquq/robroz-host"
RUNNER_IMAGE="ghcr.io/pandaquq/robroz-runner"
# Images are tagged per release and there is no `latest` tag, so the version is
# pinned here (override with ROBROZ_VERSION or --version). Do not look up
# releases/latest: v0.* tags are pre-releases and that API skips them.
# The release workflow refuses to publish a v* tag if this pin, or the tags in
# deploy/compose.host.yml, do not match the tag. Bump both in a PR, merge to
# main, then tag. The workflow never pushes to main.
DEFAULT_VERSION="v0.2.0"
SOCKET="/var/run/docker.sock"
DOCKER_SOCK_DOCS="https://github.com/PandaQuQ/robroz/blob/main/docs/docker-host.md#security-dockersock-is-root-on-this-machine"

DRY_RUN=0
NO_START=0
NO_PULL=0
DIR="${ROBROZ_DIR:-}"
VERSION="${ROBROZ_VERSION:-}"
CONTAINER_UID=""
CONTAINER_GID=""

usage() {
  cat <<'EOF'
Usage: install.sh [--dir DIR] [--version TAG] [--dry-run] [--no-start] [--no-pull]

  --dir DIR      install directory (default: $HOME/robroz; env ROBROZ_DIR)
                 path may only contain letters, digits and / . _ -
  --version TAG  image tag for host and runner (env ROBROZ_VERSION)
  --dry-run      check docker and print what would happen; write and start nothing
  --no-start     write the files, but do not pull images or start the host
  --no-pull      do not pull images; they must already be present locally
  -h, --help     show this help
EOF
}

say() { printf '%s\n' "$*"; }
# A step that changes something: prefixed in a dry run.
note() {
  if [ "$DRY_RUN" -eq 1 ]; then say "[dry-run] $*"; else say "$*"; fi
}
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Runs a command that may need root: as is when we are root, else through sudo.
as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    die "need root to run '$*', and sudo is not installed"
  fi
}

# Writes the compose file to $1. Single quotes keep ${...} for compose to read.
# CI compares this block with deploy/compose.host.yml.
write_compose() {
  sed "s/__VERSION__/${VERSION}/g" > "$1" <<'EOF'
# Written by deploy/install.sh; the next run replaces this file (the old copy is
# kept as compose.yml.bak). Put your own settings in .env instead.
#
# docker.sock is root-equivalent on this machine: see docs/docker-host.md.
secrets:
  robroz_token:
    file: ./secrets/robroz_token

services:
  host:
    image: ghcr.io/pandaquq/robroz-host:__VERSION__
    group_add:
      - "${DOCKER_GID:?set DOCKER_GID to the docker group id}"
    environment:
      ROBROZ_ADDR: "0.0.0.0:8790"
      # Same absolute path as the volume below, on both sides.
      ROBROZ_DB: "${ROBROZ_DATA:-/srv/robroz}/robroz.db"
      ROBROZ_VOLUMES: "${ROBROZ_DATA:-/srv/robroz}/groups"
      ROBROZ_CONTROL: "${ROBROZ_DATA:-/srv/robroz}/control"
      ROBROZ_IMAGE: ghcr.io/pandaquq/robroz-runner:__VERSION__
      ROBROZ_TOKEN_FILE: /run/secrets/robroz_token
      # Owner of the files the runners write under the data directory.
      ROBROZ_CONTAINER_USER: "1000:1000"
    volumes:
      - "${ROBROZ_DATA:-/srv/robroz}:${ROBROZ_DATA:-/srv/robroz}"
      - /var/run/docker.sock:/var/run/docker.sock
    ports:
      - "${ROBROZ_BIND:-127.0.0.1}:${ROBROZ_PORT:-8790}:8790"
    secrets:
      - robroz_token
    restart: unless-stopped
EOF
}

# Writes the .env template to $1. Placeholders and settings only, no credentials.
write_env() {
  cat > "$1" <<EOF
# Settings for compose.yml. Written once by deploy/install.sh; later runs leave
# this file alone. Nothing here is a credential.

# Data directory (database, group folders, control files). It is mounted at the
# same absolute path inside the container, which the runners require. Move it
# only together with its contents.
ROBROZ_DATA=${DATA}

# Group id of the docker socket on this machine.
DOCKER_GID=${DOCKER_GID}

# Where the host listens on this machine. Keep 127.0.0.1 and put an HTTPS
# reverse proxy or tunnel in front for other machines (docs/docker-host.md).
# To reach the host from a phone on this network, set ROBROZ_BIND to this
# machine's LAN or Tailscale address and run the install script again.
ROBROZ_BIND=127.0.0.1
ROBROZ_PORT=8790

# The API token is in secrets/robroz_token. The model (URL, name, key) is set in
# the client; see docs/configuration.md for the environment variables.
EOF
}

# Reads NAME's value from the .env file at $1.
env_value() {
  sed -n "s/^$2=//p" "$1" | tail -n 1
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --dry-run) DRY_RUN=1 ;;
      --no-start) NO_START=1 ;;
      --no-pull) NO_PULL=1 ;;
      --dir)
        [ $# -ge 2 ] || die "--dir needs a value"
        DIR="$2"; shift ;;
      --dir=*) DIR="${1#--dir=}" ;;
      --version)
        [ $# -ge 2 ] || die "--version needs a value"
        VERSION="$2"; shift ;;
      --version=*) VERSION="${1#--version=}" ;;
      -h|--help) usage; exit 0 ;;
      *) usage >&2; die "unknown option: $1" ;;
    esac
    shift
  done
}

check_docker() {
  if [ -n "${DOCKER_HOST:-}" ]; then
    die "DOCKER_HOST is set (${DOCKER_HOST}). This installer only uses the default engine socket ${SOCKET}, so a rootless or remote engine is not supported. Start the host by hand: ${DOCKER_SOCK_DOCS}"
  fi
  command -v docker >/dev/null 2>&1 \
    || die "docker is not installed. Install Docker Engine first: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 \
    || die "the 'docker compose' plugin is missing (the old docker-compose command is not used). See https://docs.docker.com/compose/install/linux/"
  docker info >/dev/null 2>&1 \
    || die "cannot talk to the Docker engine. Is it running, and is your user in the 'docker' group (or run this as root)?"
  if [ ! -S "$SOCKET" ]; then
    die "${SOCKET} is missing or is not a socket. This installer only uses that default engine socket, so a rootless engine (whose socket is typically /run/user/<uid>/docker.sock) is not supported. Start the host by hand: ${DOCKER_SOCK_DOCS}"
  fi
  DOCKER_GID=$(stat -c %g "$SOCKET" 2>/dev/null) || die "cannot read the group of ${SOCKET}"
  if [ "$(uname -m)" != "x86_64" ] && [ "$(uname -m)" != "amd64" ]; then
    warn "the published images are linux/amd64 only and this machine is $(uname -m)"
  fi
}

prepare_dir() {
  case "$DIR" in
    "") [ -n "${HOME:-}" ] || die "HOME is not set; pass --dir"; DIR="$HOME/robroz" ;;
  esac
  case "$DIR" in
    /*) ;;
    *) DIR="$PWD/$DIR" ;;
  esac
  # Compose files and .env are written unquoted, so keep the path plain.
  case "$DIR" in
    *[!A-Za-z0-9/._-]*) die "the install directory may only contain letters, digits and / . _ - : $DIR" ;;
  esac
  case "$VERSION" in
    "") VERSION="$DEFAULT_VERSION" ;;
  esac
  case "$VERSION" in
    *[!A-Za-z0-9._-]*) die "bad version tag: $VERSION" ;;
  esac
  DATA="$DIR/data"
}

# Sets CONTAINER_UID and CONTAINER_GID from the host image. The image USER is
# `robroz` (host/Dockerfile, uid 1000 with a matching group). compose.yml does
# not set user:, so that is the process that reads the token. group_add /
# DOCKER_GID only adds the engine socket's group and is not used for the token.
resolve_container_ids() {
  CONTAINER_UID=""
  CONTAINER_GID=""
  docker image inspect "${HOST_IMAGE}:${VERSION}" >/dev/null 2>&1 || return 1
  ids=$(docker run --rm --entrypoint /usr/bin/id "${HOST_IMAGE}:${VERSION}" 2>/dev/null) || return 1
  CONTAINER_UID=$(printf '%s\n' "$ids" | sed -n 's/^uid=\([0-9][0-9]*\).*/\1/p')
  CONTAINER_GID=$(printf '%s\n' "$ids" | sed -n 's/^uid=[^ ]* gid=\([0-9][0-9]*\).*/\1/p')
  [ -n "$CONTAINER_UID" ] && [ -n "$CONTAINER_GID" ] || return 1
}

# Token: owner is the installing user, group is the container gid, mode 0640,
# so you can cat it and the host process can read it. Data dir: owned by the
# container uid:gid, mode 0700, so the host can write and other users on this
# machine cannot read the database. The database file (and its journal, if
# present) is mode 0600. Existing file contents are not touched.
fix_access() {
  if [ "$DRY_RUN" -eq 1 ]; then
    say "[dry-run] after pull, set token owner=$(id -u) group=<container gid> mode 0640, data directory mode 0700 owned by the container uid:gid, and the database mode 0600"
    return 0
  fi
  if resolve_container_ids; then
    say "container user ${CONTAINER_UID}:${CONTAINER_GID} (from ${HOST_IMAGE}:${VERSION})"
    token="$DIR/secrets/robroz_token"
    if [ -f "$token" ]; then
      as_root chown "$(id -u):${CONTAINER_GID}" "$token"
      as_root chmod 0640 "$token"
      say "token  owner=$(id -u) group=${CONTAINER_GID} mode 0640"
    fi
  else
    if [ "$NO_START" -eq 1 ]; then
      say "token ownership: left as uid $(id -u) until the host image is pulled"
    else
      warn "could not read the user of ${HOST_IMAGE}:${VERSION}; leave token ownership as it is"
    fi
  fi
  tighten_data
}

# Mode 0700 on the data directory and 0600 on the database files. A rerun
# tightens an older install. Ownership follows the container user when known.
tighten_data() {
  [ -d "$DATA" ] || return 0
  owner=$(stat -c %u "$DATA")
  me=$(id -u)
  if [ -n "$CONTAINER_UID" ] && { [ "$owner" = "$me" ] || [ "$owner" = "$CONTAINER_UID" ]; }; then
    as_root chown "${CONTAINER_UID}:${CONTAINER_GID}" "$DATA"
  elif [ -n "$CONTAINER_UID" ] && [ "$owner" != "$me" ] && [ "$owner" != "$CONTAINER_UID" ]; then
    warn "data directory $DATA is owned by uid $owner; the container runs as ${CONTAINER_UID}:${CONTAINER_GID} and may not be able to write"
    return 0
  fi
  as_root chmod 0700 "$DATA"
  say "data   $DATA mode 0700"
  for name in robroz.db robroz.db-wal robroz.db-shm robroz.db-journal; do
    f="$DATA/$name"
    [ -f "$f" ] || continue
    if [ -n "$CONTAINER_UID" ]; then
      as_root chown "${CONTAINER_UID}:${CONTAINER_GID}" "$f"
    fi
    as_root chmod 0600 "$f"
    say "data   $f mode 0600"
  done
}

# True when $1 (an image:tag) is already on this machine.
image_present() {
  docker image inspect "$1" >/dev/null 2>&1
}

# Pull $1 unless --no-pull is set or the image is already local.
ensure_image() {
  image=$1
  if image_present "$image"; then
    say "keep   $image (already present locally)"
    return 0
  fi
  if [ "$NO_PULL" -eq 1 ]; then
    die "$image is not present locally. Load or build it first, or omit --no-pull so the script can pull it."
  fi
  say "pull   $image"
  docker pull "$image"     || die "could not pull $image. Is ${VERSION} published? Check https://github.com/${REPO}/releases, and pass --version with a tag that exists."
}

# What a dry-run would do for one image.
note_pull() {
  image=$1
  if [ "$NO_PULL" -eq 1 ]; then
    say "[dry-run] skip   pull $image (--no-pull)"
  elif image_present "$image"; then
    say "[dry-run] skip   pull $image (already present locally)"
  else
    say "[dry-run] docker pull $image"
  fi
}

install_files() {
  say "install directory: $DIR"
  say "version:           $VERSION"

  if [ "$DRY_RUN" -eq 0 ]; then
    mkdir -p "$DIR" "$DIR/secrets"
    chmod 700 "$DIR/secrets"
  else
    say "[dry-run] mkdir -p $DIR $DIR/secrets"
  fi

  # compose.yml is owned by this script: refresh it, keeping the old copy.
  compose="$DIR/compose.yml"
  if [ "$DRY_RUN" -eq 1 ]; then
    if [ -f "$compose" ]; then say "[dry-run] replace $compose"; else say "[dry-run] create $compose"; fi
  else
    tmp="$DIR/.compose.yml.new"
    write_compose "$tmp"
    if [ -f "$compose" ] && cmp -s "$tmp" "$compose"; then
      rm -f "$tmp"
      say "keep   $compose (unchanged)"
    else
      if [ -f "$compose" ]; then
        cp "$compose" "$compose.bak"
        say "update $compose (old copy: compose.yml.bak)"
      else
        say "create $compose"
      fi
      mv "$tmp" "$compose"
    fi
  fi

  # .env: only when missing.
  envfile="$DIR/.env"
  if [ -f "$envfile" ]; then
    say "keep   $envfile (exists, not touched)"
    old_gid=$(env_value "$envfile" DOCKER_GID)
    if [ -n "$old_gid" ] && [ "$old_gid" != "$DOCKER_GID" ]; then
      warn "DOCKER_GID in .env is $old_gid but the docker socket's group is $DOCKER_GID; edit .env if the host cannot reach Docker"
    fi
    DATA=$(env_value "$envfile" ROBROZ_DATA)
    [ -n "$DATA" ] || DATA="/srv/robroz"
    PORT=$(env_value "$envfile" ROBROZ_PORT)
    [ -n "$PORT" ] || PORT=8790
    BIND=$(env_value "$envfile" ROBROZ_BIND)
    [ -n "$BIND" ] || BIND=127.0.0.1
  else
    note "create $envfile"
    if [ "$DRY_RUN" -eq 0 ]; then
      write_env "$envfile"
      chmod 600 "$envfile"
    fi
    PORT=8790
    BIND=127.0.0.1
  fi

  # Token: only when missing. Generated here, never printed. Permissions are
  # finished in fix_access once the container user is known.
  token="$DIR/secrets/robroz_token"
  if [ -f "$token" ]; then
    say "keep   $token (exists, not touched)"
  else
    note "create $token (random, not printed)"
    if [ "$DRY_RUN" -eq 0 ]; then
      (umask 077 && od -An -N24 -tx1 /dev/urandom | tr -d ' \n' > "$token")
      [ -s "$token" ] || die "could not generate a token"
    fi
  fi

  # Data directory: only when missing; existing data is never touched.
  if [ -d "$DATA" ]; then
    say "keep   $DATA (exists)"
  else
    note "create $DATA"
    if [ "$DRY_RUN" -eq 0 ]; then
      mkdir -p "$DATA"
      chmod 0700 "$DATA"
    fi
  fi
}

start_host() {
  if [ "$NO_START" -eq 1 ]; then
    say "skip   pulling images and starting (--no-start)"
    fix_access
    return
  fi
  if [ "$DRY_RUN" -eq 1 ]; then
    note_pull "${RUNNER_IMAGE}:${VERSION}"
    note_pull "${HOST_IMAGE}:${VERSION}"
    fix_access
    say "[dry-run] docker compose up -d  (in $DIR)"
    return
  fi
  cd "$DIR"
  say "runner image (started per task): ${RUNNER_IMAGE}:${VERSION}"
  ensure_image "${RUNNER_IMAGE}:${VERSION}"
  say "host image: ${HOST_IMAGE}:${VERSION}"
  ensure_image "${HOST_IMAGE}:${VERSION}"
  fix_access
  say "start  docker compose up -d"
  docker compose up -d
}


wait_ready() {
  [ "$DRY_RUN" -eq 0 ] && [ "$NO_START" -eq 0 ] || return 0
  command -v curl >/dev/null 2>&1 || return 0
  say "wait   for the host on 127.0.0.1:${PORT}"
  i=0
  while [ "$i" -lt 30 ]; do
    # No token must give 401: the API is up and protected.
    code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/api/bros" 2>/dev/null || true)
    if [ "$code" = "401" ] || [ "$code" = "200" ]; then
      say "ok     the host answers (HTTP ${code} without a token)"
      return 0
    fi
    i=$((i + 1))
    sleep 1
  done
  warn "the host did not answer within 30 seconds. Look at: cd $DIR && docker compose logs host"
}

print_next_steps() {
  if [ "$DRY_RUN" -eq 1 ]; then
    say ""
    say "Dry run finished. Nothing was written or started."
    return
  fi
  token="$DIR/secrets/robroz_token"
  if [ -r "$token" ]; then
    show_token="cat $token"
  else
    show_token="sudo cat $token"
  fi
  say ""
  if [ "$NO_START" -eq 1 ]; then
    say "Files are written. Start the host with: cd $DIR && docker compose up -d"
  else
    say "robroz host ${VERSION} is running."
  fi
  say ""
  if [ "$BIND" = "127.0.0.1" ]; then where="this machine only"; else where="bound to ${BIND}, edit .env to change"; fi
  say "URL:    http://${BIND}:${PORT}  (${where})"
  say "        To connect from a phone, set ROBROZ_BIND in $DIR/.env to this"
  say "        machine's LAN or Tailscale address and run this script again."
  say "Token:  stored in $token (not printed here)"
  say "        show it with: $show_token"
  say "Data:   $DATA"
  say ""
  say "Next steps:"
  say "  1. Download the client for your computer from"
  say "     https://github.com/PandaQuQ/robroz-client/releases and check its SHA256SUMS."
  say "  2. In the client enter the URL and the token. From another machine, put an"
  say "     HTTPS reverse proxy or tunnel in front of 127.0.0.1:${PORT}"
  say "     (docs/docker-host.md); do not publish the port on 0.0.0.0."
  say "  3. Enter your model URL, name and key in the client's settings."
  say ""
  say "Upgrade: run this script again (or with --version TAG). Your .env, token and"
  say "data are kept. Logs: cd $DIR && docker compose logs -f host"
}

main() {
  parse_args "$@"
  [ "$(uname -s)" = "Linux" ] || die "this installer supports Linux only. For other setups see https://github.com/PandaQuQ/robroz/blob/main/docs/docker-host.md"
  prepare_dir
  check_docker
  install_files
  start_host
  wait_ready
  print_next_steps
}

# Everything is defined above, so a truncated download runs nothing.
main "$@"
