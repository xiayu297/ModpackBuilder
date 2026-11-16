# 术语表

本表统一 ModpackBuilder 在界面、文档、代码标识和游戏内产物中的术语。

| 中文 | English | 定义 |
| --- | --- | --- |
| 模组 | Mod | Factorio 的可加载内容单元。已安装模组可能是 zip 文件，也可能是文件夹。 |
| 模组包 / 模组包合集 | Modpack | 本工具生成的依赖容器。它本身不新增游戏内容，只声明依赖哪些成员模组。 |
| 成员模组 | Member Mod | 被用户选中、写入模组包依赖列表的已安装模组。 |
| 内置模组 / DLC 模组 | Built-in / DLC Mod | 随游戏本体提供、在 mods 目录中没有 zip 或文件夹产物的模组，例如 `base`、`space-age`、`quality`。只列出，不可选为成员。 |
| 已安装模组 | Installed Mod | mods 目录中存在 zip 或文件夹产物、且能从 `info.json` 读取元数据的模组。 |
| ZIP 模组 | ZIP Mod | 以 `<name>_<version>.zip` 形式存在的模组。zip 内顶层文件夹名不受游戏限制，`info.json` 是权威元数据。 |
| 文件夹模组 | Folder Mod | 解压到 mods 目录中的模组文件夹。 |
| 显示标题 | Title | 游戏模组列表中展示给玩家的名称，可含中文，最长 100 字符（locale 可覆盖）。 |
| 内部名 | Internal Name | 游戏用于识别模组的唯一名称，也是 zip 文件名和依赖声明的键。本工具限制为 3~50 个字符，只允许 ASCII 字母、数字、`-`、`_`，且首字符必须是字母或数字。 |
| 模组包版本 | Modpack Version | 模组包自身的版本号，格式必须是三段数字，例如 `1.0.0`，每段 0~65535。 |
| 依赖约束 | Dependency Constraint | 写入 `info.json` 的依赖声明，例如 `flib >= 0.17.2`。本工具使用 `>=`。 |
| 合集标识文件 | Marker | 生成 zip 根目录下的 `module-set-builder.json`，记录工具版本、生成时间、成员列表，用于识别、编辑和删除模组包。 |
| 追加注册 | Append Registration | 只向 `mod-list.json` 的 `mods` 数组末尾插入新条目，不改动任何已有字节。 |
| 原格式插入 | Byte-preserving Insert | 保留文件已有缩进、空行、换行符、BOM 和字段顺序，仅在数组末尾插入新对象。 |
| 静态兼容性检查 | Static Compatibility Check | 仅依据各模组 `info.json` 的 `factorio_version` 和 `dependencies` 声明做检查，不执行游戏代码，无法保证运行时完全兼容。 |
| 备份 | Backup | 写入前复制到工具目录 `backups/` 的原文件，默认保留 7 天。 |
| 扫描 | Scan | 每次启动及每次增删改后重新读取 mods 目录、`mod-list.json` 和 zip 内 `info.json`。 |
| 游戏版本 | Game Version | 模组在 `info.json` 的 `factorio_version` 字段中声明的目标游戏大版本。本工具只面向 Factorio 2.1，不读取 `factorio-current.log`。 |
