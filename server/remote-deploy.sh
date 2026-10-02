#!/usr/bin/env bash
# Poymai Deploy — server side release script.
#
# The deploy app runs it from the project checkout on the server:
#   cd <repo> && git fetch -q origin && f=$(mktemp) && git show <sha>:scripts/deploy/remote-deploy.sh > "$f" \
#     && bash "$f" deploy <sha>; rc=$?; rm -f "$f"; exit $rc
# so the script that installs a release is always the one from that release.
# (Not piped into `bash -s`: docker/git children could swallow the rest of stdin.)
#
# Commands:
#   deploy <sha> [--dry-run] [--accept-drift] [--backup-db]
#   status                      one JSON line with the current release
#   releases [N]                last N releases (JSON lines)
#
# Output protocol (parsed by the app, everything else is plain log):
#   ::step <id>|<text>   ::warn <text>   ::error <code>|<text>   ::result <json>
#
# Exit codes: 0 ok, 10 busy, 11 unknown commit, 20 drift, 30 validate failed,
#             31 build failed, 40 health failed (rolled back), 41 rollback failed.
set -Eeuo pipefail

REPO_DIR="$(pwd)"
STATE_DIR="$REPO_DIR/.deploy"
LOCK_FILE="$STATE_DIR/lock"
CURRENT_FILE="$STATE_DIR/current.json"
RELEASES_FILE="$STATE_DIR/releases.jsonl"
mkdir -p "$STATE_DIR"

ACTOR="${DEPLOY_ACTOR:-unknown}"
MACHINE="${DEPLOY_MACHINE:-unknown}"

step()  { printf '::step %s|%s\n' "$1" "$2"; }
warn()  { printf '::warn %s\n' "$*"; }
fail()  { printf '::error %s|%s\n' "$1" "$2"; exit "$1"; }
now()   { date -u +%Y-%m-%dT%H:%M:%SZ; }
json_escape() { python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().rstrip("\n")))'; }

# ── Project configuration (overridable in scripts/deploy/deploy.conf) ─────────
COMPOSE_SERVICES=""        # services to build/up; empty = whole project
HEALTH_URLS=""             # space separated; each must answer < 500
HEALTH_TIMEOUT=150         # seconds
KEEP_DB_BACKUPS=10
PRUNE_BUILD_CACHE=1
DB_BACKUP_PATHS=""         # regex of paths whose change triggers a DB dump
deploy_validate() { :; }   # runs on the new checkout before building
deploy_db_backup() { :; }  # writes a DB dump to $1
deploy_post_up() { :; }    # $1 = previous sha, $2 = new sha

load_config() {
    local sha="$1" conf
    if conf="$(git show "$sha:scripts/deploy/deploy.conf" 2>/dev/null)"; then
        # shellcheck disable=SC1090
        source <(printf '%s\n' "$conf")
    fi
}

compose() {
    if docker compose version >/dev/null 2>&1; then docker compose "$@"; else docker-compose "$@"; fi
}

current_sha() { git rev-parse --verify -q HEAD || true; }

write_status() {
    local sha="$1" status="$2" started="$3" prev="$4" msg
    msg="$(git log -1 --format=%s "$sha" 2>/dev/null | json_escape)"
    local line
    line=$(printf '{"sha":"%s","prev":"%s","status":"%s","actor":%s,"machine":%s,"started":"%s","finished":"%s","message":%s}' \
        "$sha" "$prev" "$status" "$(printf %s "$ACTOR" | json_escape)" "$(printf %s "$MACHINE" | json_escape)" \
        "$started" "$(now)" "$msg")
    printf '%s\n' "$line" >> "$RELEASES_FILE"
    if [[ "$status" == "success" || "$status" == "rolled_back" ]]; then
        printf '%s\n' "$line" > "$CURRENT_FILE"
    fi
    printf '::result %s\n' "$line"
}

# ── Health ────────────────────────────────────────────────────────────────────
containers_ok() {
    # Every container of the project must be running and not unhealthy.
    local bad
    bad="$(compose ps --format '{{.Service}} {{.State}} {{.Health}}' $COMPOSE_SERVICES 2>/dev/null \
        | awk '$2 != "running" || $3 == "unhealthy" || $3 == "starting" {print}')"
    [[ -z "$bad" ]]
}

urls_ok() {
    local url code
    for url in $HEALTH_URLS; do
        code="$(curl -ks -o /dev/null -w '%{http_code}' --max-time 5 "$url" || echo 000)"
        if [[ "$code" == 000 || "$code" -ge 500 ]]; then return 1; fi
    done
}

wait_healthy() {
    if [[ "${POYMAI_FORCE_HEALTH_FAIL:-0}" == 1 ]]; then
        warn "POYMAI_FORCE_HEALTH_FAIL=1 — simulating a failed health check"
        return 1
    fi
    local deadline=$((SECONDS + HEALTH_TIMEOUT))
    while (( SECONDS < deadline )); do
        if urls_ok && containers_ok; then return 0; fi
        sleep 3
    done
    compose ps $COMPOSE_SERVICES || true
    compose logs --tail=60 $COMPOSE_SERVICES || true
    return 1
}

# ── Drift: local edits on the server never silently disappear ────────────────
save_drift() {
    local branch="drift/$(date -u +%Y%m%d-%H%M%S)-$RANDOM" idx tree commit
    idx="$(mktemp)"
    GIT_INDEX_FILE="$idx" git read-tree HEAD
    GIT_INDEX_FILE="$idx" git add -A
    tree="$(GIT_INDEX_FILE="$idx" git write-tree)"
    rm -f "$idx"
    commit="$(git -c user.name="Poymai Server" -c user.email="server@poymai.local" \
        commit-tree "$tree" -p HEAD -m "Server drift captured before deploy")"
    git push -q origin "$commit:refs/heads/$branch"
    echo "$branch"
}

# ── Commands ──────────────────────────────────────────────────────────────────
cmd_status() {
    local holder="" drift
    if [[ -f "$LOCK_FILE" ]] && ! flock -n "$LOCK_FILE" true 2>/dev/null; then
        holder="$(cat "$STATE_DIR/lock.owner" 2>/dev/null || true)"
    fi
    drift="$(git status --porcelain | wc -l)"
    printf '{"head":"%s","drift":%s,"busy":%s,"current":%s}\n' \
        "$(current_sha)" "$drift" "$(printf %s "$holder" | json_escape)" \
        "$(cat "$CURRENT_FILE" 2>/dev/null || echo null)"
}

cmd_releases() {
    tail -n "${1:-20}" "$RELEASES_FILE" 2>/dev/null || true
}

cmd_deploy() {
    local sha="" dry=0 accept_drift=0 force_backup=0
    while (($#)); do
        case "$1" in
            --dry-run) dry=1 ;;
            --accept-drift) accept_drift=1 ;;
            --backup-db) force_backup=1 ;;
            *) sha="$1" ;;
        esac
        shift
    done
    [[ -n "$sha" ]] || fail 11 "No commit given"
    local started; started="$(now)"

    step lock "Блокировка деплоя"
    exec 9>"$LOCK_FILE"
    if ! flock -n 9; then
        fail 10 "Уже идёт деплой: $(cat "$STATE_DIR/lock.owner" 2>/dev/null || echo '?')"
    fi
    printf '%s (%s) since %s' "$ACTOR" "$MACHINE" "$started" > "$STATE_DIR/lock.owner"
    trap 'rm -f "$STATE_DIR/lock.owner"' EXIT

    step fetch "Получение коммита с GitHub"
    git fetch -q origin --prune
    sha="$(git rev-parse --verify -q "$sha^{commit}")" || fail 11 "Коммит не найден на GitHub"
    if [[ -z "$(git branch -r --contains "$sha" 2>/dev/null)" ]]; then
        fail 11 "Коммит $sha не запушен на GitHub — деплой разрешён только из GitHub"
    fi
    load_config "$sha"
    local prev; prev="$(current_sha)"
    echo "Текущий: ${prev:0:8} → новый: ${sha:0:8}"

    step drift "Проверка ручных правок на сервере"
    if [[ -n "$(git status --porcelain)" ]]; then
        git status --short | head -30
        local branch; branch="$(save_drift)"
        if (( ! accept_drift )); then
            fail 20 "На сервере есть ручные правки. Они сохранены в ветку $branch. Перенесите их в код или запустите деплой с «принять дрейф»."
        fi
        warn "Ручные правки сохранены в $branch и будут перезаписаны"
        git reset -q --hard
        git clean -fdq   # untracked only; ignored files (.env, certs, data) stay
    fi

    if (( dry )); then
        step validate "Проверка (dry-run)"
        local tmp; tmp="$(mktemp -d)"
        git worktree add -q --detach "$tmp" "$sha"
        (cd "$tmp" && deploy_validate) || { git worktree remove --force "$tmp"; fail 30 "Проверка не прошла"; }
        git worktree remove --force "$tmp"
        echo "Изменения:"; git diff --stat "$prev" "$sha" | tail -20
        write_status "$sha" "dry_run" "$started" "$prev"
        return 0
    fi

    if (( force_backup )) || { [[ -n "$DB_BACKUP_PATHS" && -n "$prev" ]] && git diff --name-only "$prev" "$sha" | grep -Eq "$DB_BACKUP_PATHS"; }; then
        step backup "Резервная копия базы данных"
        mkdir -p "$REPO_DIR/backups"
        local dump="$REPO_DIR/backups/db-$(date -u +%Y%m%d-%H%M%S)-${prev:0:8}.sql.gz"
        deploy_db_backup "$dump"
        echo "Бэкап: $dump ($(du -h "$dump" | cut -f1))"
        ls -1t "$REPO_DIR"/backups/db-*.sql.gz 2>/dev/null | tail -n +$((KEEP_DB_BACKUPS + 1)) | xargs -r rm -f
    fi

    step checkout "Переключение на новую версию"
    git checkout -q --detach --force "$sha"

    step validate "Проверка кода"
    if ! deploy_validate; then
        [[ -n "$prev" ]] && git checkout -q --detach --force "$prev"
        fail 30 "Проверка кода не прошла — сервер остался на ${prev:0:8}"
    fi

    step build "Сборка Docker-образов"
    if ! compose build $COMPOSE_SERVICES; then
        [[ -n "$prev" ]] && git checkout -q --detach --force "$prev"
        write_status "$sha" "build_failed" "$started" "$prev"
        fail 31 "Сборка не удалась — работающие контейнеры не тронуты"
    fi

    step up "Перезапуск изменённых контейнеров"
    compose up -d --remove-orphans $COMPOSE_SERVICES
    deploy_post_up "$prev" "$sha"

    step health "Проверка здоровья сервисов"
    if wait_healthy; then
        if (( PRUNE_BUILD_CACHE )); then docker builder prune -af >/dev/null 2>&1 || true; fi
        step done "Готово"
        write_status "$sha" "success" "$started" "$prev"
        return 0
    fi

    if [[ -z "$prev" ]]; then
        write_status "$sha" "failed" "$started" "$prev"
        fail 41 "Сервисы не поднялись, предыдущей версии нет"
    fi
    step rollback "Откат на ${prev:0:8}"
    git checkout -q --detach --force "$prev"
    load_config "$prev"
    compose build $COMPOSE_SERVICES && compose up -d --remove-orphans $COMPOSE_SERVICES && deploy_post_up "$sha" "$prev"
    if POYMAI_FORCE_HEALTH_FAIL=0 wait_healthy; then
        write_status "$prev" "rolled_back" "$started" "$sha"
        fail 40 "Новая версия не прошла проверку — сервер откатан на ${prev:0:8}"
    fi
    write_status "$prev" "rollback_failed" "$started" "$sha"
    fail 41 "КРИТИЧНО: откат тоже не прошёл проверку здоровья"
}

case "${1:-}" in
    deploy) shift; cmd_deploy "$@" ;;
    status) cmd_status ;;
    releases) shift; cmd_releases "${1:-20}" ;;
    *) echo "usage: remote-deploy.sh deploy <sha> [--dry-run] [--accept-drift] [--backup-db] | status | releases [N]" >&2; exit 2 ;;
esac
