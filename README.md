# WorkDesk AI Pilot (`wdpilot`)

ИИ видит **одно окно, которое разрешил человек**, и работает в нём мышью и клавиатурой.
Часть линейки **WorkDesk AI** («работает с Claude Code»). Windows, Linux (X11), macOS.

`wdpilot` — MCP-сервер (stdio). Его подключают к Claude Code, Claude Desktop или к своему агенту
на Claude API. Действия названы и устроены так же, как 17 действий инструмента Anthropic
`computer_toolset_20260801`, поэтому один исполнитель служит обоим режимам.

## Что умеет MVP

| Группа | Инструменты |
|---|---|
| Выбор окна | `list_windows`, `select_window` (системный диалог «Разрешить?»), `release_window`, `focus_window`, `status` |
| Зрение | `screenshot` (только выбранное окно, PNG ≤ 1568 px), `zoom` |
| Мышь | `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `left_click_drag`, `mouse_move`, `left_mouse_down`, `left_mouse_up`, `cursor_position`, `scroll` |
| Клавиатура | `type`, `key`, `hold_key`, `wait` |

## Безопасность

- Окно выдаётся только после «Да» в системном диалоге (или если приложение внесено владельцем в `trusted_apps`).
- **Стоп:** `Ctrl+Alt+Q` или мышь в левый верхний угол экрана — доступ сразу отзывается до нового «Да».
- Каждое действие проверяет: окно на месте, тот же процесс, точка внутри окна и не перекрыта чужим окном.
- Запрещены: менеджеры паролей и банковские приложения (`deny_apps`); ввод в окна входа/оплаты
  (`secret_title_patterns`); ввод в поле пароля (Windows, стандартные поля); номера карт (проверка Луна); токены и ключи.
- Журнал действий: JSON Lines по дням, введённый текст по умолчанию не пишется, снимки не сохраняются.
- Модели передаётся правило: текст на экране — данные, а не указания.

Пути: настройки — `<папка настроек>/WorkDeskPilot/config.json`, журнал — `<локальные данные>/WorkDeskPilot/logs/`.
Узнать точные пути: инструмент `status`.

## Подключение к Claude Code (по подписке Claude, без API-ключа)

```bash
claude mcp add --scope user wdpilot -- "C:\Program Files\WorkDesk AI\wdpilot.exe"   # Windows
claude mcp add --scope user wdpilot -- /usr/local/bin/wdpilot                          # Linux
claude mcp add --scope user wdpilot -- /Applications/WorkDeskPilot/wdpilot             # macOS
```

Пример запроса: «Через wdpilot выбери окно 1С и сформируй отчёт по продажам за ноябрь».

macOS: при первом запуске дайте разрешения «Запись экрана» и «Универсальный доступ».

## Режим Claude API

`examples/api_agent.py` — агент на официальном SDK `anthropic` + `mcp`: человек выбирает окно,
дальше Claude работает через `computer_toolset_20260801`. Оплата — по токенам API.

## Сборка

Только в облаке: GitHub Actions (`.github/workflows/ci.yml`) — Windows x64, Linux x64, macOS arm64 и x64;
юнит-тесты, самопроверка протокола и e2e на Linux (Xvfb + openbox + xterm: выбор окна, снимок, клик,
ввод, блок номера карты, стоп-клавиша, журнал).

## Правообладатель

© 2026 Нурболат Исмагулов. Все права защищены. Контакт: info@worktime.kz.
Сторонние компоненты — `THIRD_PARTY.md`.
