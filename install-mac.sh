#!/usr/bin/env bash
# Poymai Deploy — one-shot installer for macOS.
# Run from the cloned repo:  bash ~/code/poymai-deploy/install-mac.sh
# Safe to run again: every step is skipped if it is already done.
set -uo pipefail

CODE="$HOME/code"
KEY="$HOME/.ssh/id_ed25519_poymai"
MAIN_LAN="poymai@192.168.3.209"
MAIN_WAN="poymai@185.33.228.250"
TG_LAN="root@192.168.3.99"

say()  { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
ok()   { printf '    \033[32m✓ %s\033[0m\n' "$*"; }
die()  { printf '\n\033[1;31m✗ %s\033[0m\n' "$*"; exit 1; }

[[ "$(uname)" == "Darwin" ]] || die "Этот скрипт только для macOS"

say "1/7  Инструменты Apple (Xcode Command Line Tools)"
if ! xcode-select -p >/dev/null 2>&1; then
  xcode-select --install
  die "Откроется окно установки. Дождитесь конца установки и запустите этот скрипт ещё раз."
fi
ok "установлены"

say "2/7  Homebrew, Node.js, Rust"
if ! command -v brew >/dev/null; then
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" || die "Не удалось поставить Homebrew"
fi
[[ -x /opt/homebrew/bin/brew ]] && eval "$(/opt/homebrew/bin/brew shellenv)"
[[ -x /usr/local/bin/brew ]] && eval "$(/usr/local/bin/brew shellenv)"
brew list git  >/dev/null 2>&1 || brew install git
brew list node >/dev/null 2>&1 || brew install node
brew list gh   >/dev/null 2>&1 || brew install gh
if ! command -v cargo >/dev/null && [[ ! -x "$HOME/.cargo/bin/cargo" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal || die "Не удалось поставить Rust"
fi
# shellcheck disable=SC1091
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
ok "готово"

say "3/7  Вход в GitHub"
if ! gh auth status >/dev/null 2>&1; then
  gh auth login -h github.com -p https -w || die "Вход в GitHub не удался"
fi
gh auth setup-git
git config --global user.name  >/dev/null || git config --global user.name  "Lonkbot228"
git config --global user.email >/dev/null || git config --global user.email "Lonkbot228@users.noreply.github.com"
ok "вы вошли как $(gh api user -q .login)"

say "4/7  SSH-ключ для серверов"
mkdir -p "$HOME/.ssh" && chmod 700 "$HOME/.ssh"
[[ -f "$KEY" ]] || ssh-keygen -t ed25519 -N "" -C "poymai-deploy-mac" -f "$KEY" -q
SSH_OPTS=(-o ConnectTimeout=6 -o StrictHostKeyChecking=accept-new)

can_login() { ssh -i "$KEY" -o BatchMode=yes -o IdentitiesOnly=yes "${SSH_OPTS[@]}" "$@" true 2>/dev/null; }
install_key() { # install_key <label> <ssh args...>
  local label="$1"; shift
  if can_login "$@"; then ok "${label} — ключ уже установлен"; return 0; fi
  echo "    Введите пароль от сервера «${label}» (символы не отображаются):"
  ssh "${SSH_OPTS[@]}" -o PubkeyAuthentication=no "$@" \
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys" \
    < "$KEY.pub" && can_login "$@"
}

MAIN_OK=0
for target in "$MAIN_LAN" "$MAIN_WAN"; do
  if nc -z -G 3 "${target#*@}" 22 2>/dev/null; then
    install_key "PoymAI ($target)" "$target" && { MAIN_OK=1; break; }
  fi
done
[[ $MAIN_OK == 1 ]] || die "Не удалось подключиться к серверу PoymAI. Подключитесь к домашней сети/интернету и запустите скрипт снова."

if nc -z -G 3 192.168.3.99 22 2>/dev/null; then
  install_key "Telegram-шлюз (LAN)" "$TG_LAN" || echo "    ! шлюз пропущен — повторите позже"
else
  install_key "Telegram-шлюз (через PoymAI)" -J "$MAIN_WAN" "$TG_LAN" || echo "    ! шлюз пропущен — повторите позже"
fi

say "5/7  Проекты в $CODE"
mkdir -p "$CODE"
for repo in poymai poymaitelegram poymai-deploy; do
  if [[ -d "${CODE}/${repo}/.git" ]]; then
    git -C "${CODE}/${repo}" pull --ff-only -q 2>/dev/null && ok "${repo} обновлён" || ok "${repo} уже есть (есть локальные правки — не трогаю)"
  else
    gh repo clone "Lonkbot228/$repo" "${CODE}/${repo}" -- -q 2>/dev/null && ok "${repo} скачан" || echo "    ! не удалось скачать ${repo} (проверьте, что репозиторий существует)"
  fi
done
[[ -d "$CODE/poymai-deploy/.git" ]] || die "Нет репозитория poymai-deploy на GitHub"

say "6/7  Сборка приложения (5–10 минут при первом запуске)"
cd "$CODE/poymai-deploy" || die "нет папки приложения"
npm ci --silent || die "npm ci не удался"
npx tauri build --bundles app || die "Сборка не удалась"
APP="$(ls -d src-tauri/target/release/bundle/macos/*.app | head -1)"
[[ -d "$APP" ]] || die "Не найдено собранное приложение"

say "7/7  Установка и запуск"
pkill -f "Poymai Deploy" 2>/dev/null || true
rm -rf "/Applications/Poymai Deploy.app"
cp -R "$APP" /Applications/
xattr -cr "/Applications/Poymai Deploy.app"
open "/Applications/Poymai Deploy.app"
ok "Poymai Deploy установлен в «Программы» и запущен"

cat <<'EOF'

Готово! Значок приложения — в строке меню сверху (справа).
 • Клик по значку — открыть окно, «Задеплоить» — выкатить на сервер.
 • Приложение само запускается при входе в систему.
 • macOS спросит про уведомления — нажмите «Разрешить».
EOF
