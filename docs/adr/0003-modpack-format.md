# ADR 0003：生成的模组包格式

- 状态：已接受
- 日期：2026-11-15

## 决策

生成产物为 zip，文件名 `<内部名>_<版本>.zip`，zip 内顶层文件夹使用同名 `<内部名>_<版本>/`。

目录结构：

```
<内部名>_<版本>.zip
└── <内部名>_<版本>/
    ├── info.json
    ├── changelog.txt
    ├── module-set-builder.json
    └── locale/
        ├── en/modpack.cfg
        └── zh-CN/modpack.cfg
```

关键规则：

1. `info.json` 是唯一必需的游戏文件；**不生成任何 `.lua`**（data/control/settings 都不需要）。
2. `dependencies` 以 `base >= 2.1.0` 开头，随后是每个成员模组的 `name >= version`，按名称排序。
3. `version` 由用户输入，默认 `1.0.0`，必须是三段 0~65535 的数字。
4. `author` 默认 `ModpackBuilder`，可在创建/编辑时修改。
5. `factorio_version` 固定 `"2.1"`。
6. `title` 为用户输入的显示标题，包括可选前缀在内最长 100 字符；可选择添加 `⚙️` 前缀，保存为 `⚙️ 标题`，不影响内部名或加载顺序；`locale` 的 `[mod-name]` 再次提供同名标题。
7. `locale/en` 与 `locale/zh-CN` 分别提供英文/中文描述；`info.json.description` 同时包含英文和中文说明。
8. `changelog.txt` 遵循 Factorio 官方 changelog 格式，并列出成员模组及版本。
9. `module-set-builder.json` 是合集标识，包含 schema、工具版本、生成时间、合集元数据和成员列表；游戏会忽略该未知文件。
10. 不生成 `thumbnail.png`。
11. 分享时只传这个 zip；它不包含成员模组。接收者需要自行安装成员，缺失时 Factorio 会提示依赖问题，可从 mod portal 下载的成员可由游戏补装。

## 后果

- 生成物与用户现有的 zip 模组形式一致，游戏内删除行为与普通模组一致。
- 不写 Lua 彻底规避了 AGMP 在 data 阶段调用 control API 导致加载崩溃的问题。
- 合集 zip 很小，便于传播，但不是“一键整包分享”。
