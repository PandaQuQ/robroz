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
#   4. pulls the host and runner images and runs `docker compose up -d`;
#   5. prints the URL and the next steps.
#
# A rerun upgrades: it pulls the images for the version this script (or
# --version) names and recreates the container. It never overwrites .env, the
# token, or anything under data/.
#
# It asks for no credentials and writes none. The only secret it makes is the
# API token, generated locally and stored in secrets/robroz_token (mode 0600).
# The model account is entered later, in the client.
#
# Read docs/docker-host.md first: the host container gets docker.sock, which is
# root-equivalent on this machine.

set -eu

REPO="PandaQuQ/robroz"
HOST_IMAGE="ghcr.io/pandaquq/robroz-host"
RUNNER_IMAGE="ghcr.io/pandaquq/robroz-runner"
# Images are tagged per release and there is no `latest` tag, so the version is
# pinned here. Bump it in the same change as deploy/compose.host.yml.
DEFAULT_VERSION="v0.2.0"
# uid:gid of the `robroz` user inside the host image.
CONTAINER_UID=1000
CONTAINER_GID=1000
SOCKET="/var/run/docker.sock"

DRY_RUN=0
NO_START=0
DIR="${ROBROZ_DIR:-}"
VERSION="${ROBROZ_VERSION:-}"

usage() {
  cat <<'EOF'
Usage: install.sh [--dir DIR] [--version TAG] [--dry-run] [--no-start]

  --dir DIR      install directory (default: $HOME/robroz; env ROBROZ_DIR)
  --version TAG  image tag for host and runner (env ROBROZ_VERSION)
  --dry-run      check docker and print what would happen; write and start nothing
  --no-start     write the files, but do not pull images or start the host
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
  [ -z "${DOCKER_HOST:-}" ] \
    || die "DOCKER_HOST is set ($DOCKER_HOST). This installer supports the default socket ${SOCKET} only; for rootless Docker see docs/docker-host.md"
  command -v docker >/dev/null 2>&1 \
    || die "docker is not installed. Install Docker Engine first: https://docs.docker.com/engine/install/"
  docker compose version >/dev/null 2>&1 \
    || die "the 'docker compose' plugin is missing (the old docker-compose command is not used). See https://docs.docker.com/compose/install/linux/"
  docker info >/dev/null 2>&1 \
    || die "cannot talk to the Docker engine. Is it running, and is your user in the 'docker' group (or run this as root)?"
  [ -S "$SOCKET" ] \
    || die "${SOCKET} is not a socket. This installer supports the default socket only; see docs/docker-host.md"
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

# Creates $1 owned by the container user, unless it already exists.
make_owned_dir() {
  if [ -d "$1" ]; then
    say "keep   $1 (exists)"
    return
  fi
  note "create $1"
  if [ "$DRY_RUN" -eq 1 ]; then
    return
  fi
  mkdir -p "$1"
  if [ "$(id -u)" -ne "$CONTAINER_UID" ]; then
    as_root chown "${CONTAINER_UID}:${CONTAINER_GID}" "$1"
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

  # Token: only when missing. Generated here, never printed.
  token="$DIR/secrets/robroz_token"
  if [ -f "$token" ]; then
    say "keep   $token (exists, not touched)"
  else
    note "create $token (random, mode 0600)"
    if [ "$DRY_RUN" -eq 0 ]; then
      (umask 077 && od -An -N24 -tx1 /dev/urandom | tr -d ' \n' > "$token")
      [ -s "$token" ] || die "could not generate a token"
      # The host container runs as uid 1000 and must be able to read it.
      if [ "$(id -u)" -ne "$CONTAINER_UID" ]; then
        as_root chown "${CONTAINER_UID}:${CONTAINER_GID}" "$token"
      fi
    fi
  fi

  # Data directory: only when missing; existing data is never touched.
  make_owned_dir "$DATA"
}

start_host() {
  if [ "$NO_START" -eq 1 ]; then
    say "skip   pulling images and starting (--no-start)"
    return
  fi
  if [ "$DRY_RUN" -eq 1 ]; then
    say "[dry-run] docker pull ${RUNNER_IMAGE}:${VERSION}"
    say "[dry-run] docker compose pull   (in $DIR)"
    say "[dry-run] docker compose up -d  (in $DIR)"
    return
  fi
  cd "$DIR"
  say "pull   ${RUNNER_IMAGE}:${VERSION} (the runner image, started per task)"
  docker pull "${RUNNER_IMAGE}:${VERSION}" \
    || die "could not pull ${RUNNER_IMAGE}:${VERSION}. Is ${VERSION} published? Check https://github.com/${REPO}/releases, and pass --version with a tag that exists."
  say "pull   ${HOST_IMAGE}:${VERSION}"
  docker compose pull \
    || die "could not pull ${HOST_IMAGE}:${VERSION}. Is ${VERSION} published? Check https://github.com/${REPO}/releases, and pass --version with a tag that exists."
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
  if [ "$(id -u)" -eq "$CONTAINER_UID" ]; then
    show_token="cat $DIR/secrets/robroz_token"
  else
    show_token="sudo cat $DIR/secrets/robroz_token"
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
  say "Token:  stored in $DIR/secrets/robroz_token (not printed here)"
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
  [ "$(uname -s)" = "Linux" ] || die "this installer supports Linux only (see docs/docker-host.md for other setups)"
  prepare_dir
  check_docker
  install_files
  start_host
  wait_ready
  print_next_steps
}

# Everything is defined above, so a truncated download runs nothing.
main "$@"
