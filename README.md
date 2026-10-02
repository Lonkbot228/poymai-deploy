# Poymai Deploy 3

Трей-приложение (Windows + macOS) для деплоя PoymAI и Telegram-шлюза.

**Модель:** GitHub `main` — единственный источник правды.
Деплой = коммит → push в GitHub → сервер ставит *ровно этот коммит*
(`scripts/deploy/remote-deploy.sh` в каждом проекте) → проверка здоровья →
при ошибке автоматический откат → ветка `production` и тег `deploy/*` в GitHub.
Второй компьютер каждые 60 с видит новый коммит и сам подтягивает его
(если есть локальные правки — спрашивает).

## Установка
- Windows: `src-tauri/target/release/bundle/nsis/Poymai Deploy_*_x64-setup.exe`
- Mac: см. [MAC_SETUP.md](MAC_SETUP.md) — 4 команды, всё остальное делает `install-mac.sh`.

На каждом компьютере нужен SSH-ключ `~/.ssh/id_ed25519_poymai`, добавленный
на `poymai@192.168.3.209` и `root@192.168.3.99`.

Настройки компьютера: `~/.poymai-deploy/config.json` (пути к проектам, маршруты:
LAN → внешний `185.33.228.250` → шлюз через ProxyJump).

## CLI
```
poymai-deploy deploy main -m "сообщение"
poymai-deploy deploy telegram --dry-run
poymai-deploy deploy main --sha <commit>     # откат / передеплой
```

## Разработка
```
npm ci
npx tauri dev
```
`server/remote-deploy.sh` — эталон серверного скрипта; копия лежит в
`scripts/deploy/` каждого проекта вместе с `deploy.conf`.
`legacy/` — старое Python-приложение.
