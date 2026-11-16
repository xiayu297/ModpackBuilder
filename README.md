# ModpackBuilder

ModpackBuilder（模组包创建器）是一个便携式 Windows 命令行工具，用于为 Factorio 2.1 创建、编辑和删除模组包。模组包只声明成员依赖，不包含成员模组本体，也不生成 Lua 脚本。

界面支持 English 和简体中文。

## 功能

- 扫描 mods 目录顶层的 ZIP 和文件夹模组，读取 `info.json`。
- 同一内部名存在多个版本时选择最高版本，并显示扫描警告。
- 检查 Factorio 版本、成员冲突及依赖问题；静态警告不保证运行时兼容。
- 依赖约束版本支持 `x.y` 和 `x.y.z`，两段形式按 `x.y.0` 比较；依赖名称可含空格。`base >= 2.1` 等内置依赖不再误报，`! Squeak Through` 等冲突会参与选中和已安装检查。
- 生成包含 `info.json`、`changelog.txt`、合集标识和英文/中文 locale 的 ZIP。
- 将新模组包以禁用状态追加到 `mod-list.json`，不改动成员模组的启用状态。
- 编辑时保留已有 `mod-list.json` 条目的启用状态；条目缺失时补为禁用。
- 写入前备份模组包和 `mod-list.json`，默认清理 7 天前的备份。
- 检测 Factorio 正在运行时拒绝扫描或写入。

## 运行

1. 完全关闭 Factorio。
2. 将 `ModpackBuilder.exe` 放到一个可写目录。无需安装器，也不会写注册表。
3. 双击 exe，或在 Windows Terminal / PowerShell 中运行：

   ```powershell
   .\ModpackBuilder.exe
   ```

默认 mods 目录为 `%APPDATA%\Factorio\mods`。首次启动会要求选择语言，并保存到 exe 同目录的 `config.json`。

程序运行时只会写入：

- Factorio mods 目录；
- exe 所在目录中的 `config.json` 和 `backups/`。

因此不要把 exe 放在只读目录中。

### 命令行选项

```text
--mods-dir <DIR>    指定 Factorio mods 目录
--lang <LANG>       临时指定界面语言：en 或 zh-CN
--backup-days <DAYS> 临时指定备份保留天数；0 表示不自动清理
--help               显示帮助
--version            显示版本
```

示例：

```powershell
.\ModpackBuilder.exe --mods-dir D:\Factorio\mods --lang zh-CN --backup-days 7
```

`--lang` 和 `--backup-days` 仅对本次运行生效，不改写已保存配置。也可在 exe 同目录的 `config.json` 中设置默认值：

```json
{
  "lang": "zh-CN",
  "backup_days": 7
}
```

### 设置

主菜单的“设置”仅提供“界面语言”和“备份保留天数”两项，修改后保存到 `config.json`。保留天数直接输入非负整数，`0` 表示不自动清理。

未使用对应命令行参数时，语言立即切换，保留天数立即生效。如果启动时显式使用了 `--lang` 或 `--backup-days`，对应参数在本次运行中始终优先；设置中保存的新值作为以后不带该参数启动时的默认值。

修改保留天数不会立即删除备份，仍按现有时机在启动及成功增删改后清理。

### 基本操作

- 方向键移动，Enter 确认。
- 成员多选时输入文字过滤，Space 选择或取消选择，Enter 确认。
- 在创建、编辑或删除中按 Esc 返回主菜单；在主菜单按 Esc 退出。
- 创建时必须选择至少一个非内置成员，内部名为 3~50 个 ASCII 字母、数字、`-` 或 `_`，首字符必须是字母或数字。
- 创建和编辑时，输入标题后可单次选择是否为显示标题添加 `⚙️` 前缀，效果为 `⚙️ 标题`；选择“否”会移除已有前缀。该选择不保存为设置，不影响内部名或加载顺序。
- 新内部名必须唯一；已有模组包请使用“编辑”，创建流程不会覆盖或自动递增版本。
- 新模组包默认禁用，需要在 Factorio 的模组界面手动启用。

## 生成的模组包

文件名为 `<内部名>_<版本>.zip`，ZIP 内包含：

```text
<内部名>_<版本>/
├── info.json
├── changelog.txt
├── module-set-builder.json
└── locale/
    ├── en/modpack.cfg
    └── zh-CN/modpack.cfg
```

`module-set-builder.json` 是 ModpackBuilder 用来识别、编辑和删除模组包的标识文件，请保留它。工具只能编辑和删除带有效标识的 ZIP 模组包；文件夹形式只能识别。

分享时只需发送模组包 ZIP，但接收者必须自行安装全部成员及依赖。模组包不包含成员模组，不生成 `.lua` 或 `thumbnail.png`。

## mod-list.json 安全策略

工具不会重新序列化整个 `mod-list.json`。追加和删除条目时保留原有 BOM、换行、缩进、空行及其他字段顺序，写入前重新解析校验，并使用同目录临时文件原子替换。

- 创建：在 `mods` 数组末尾追加一个 `enabled: false` 条目。
- 编辑：保留已有条目的启用状态；只有条目缺失时才追加禁用条目。
- 删除：只移除带有效标识的模组包对应条目。
- 遇到非法 JSON 或无法识别的结构时中止，不使用整文件重写兜底。

## 备份与恢复

备份保存在 exe 同目录的 `backups/`，命名格式为：

```text
mod-list.json.<时间戳>.bak
<内部名>_<版本>.zip.<时间戳>.bak
```

默认在启动及成功写操作后清理超过 7 天的 `.bak` 文件；`--backup-days 0` 可禁用自动清理。

手工恢复前请关闭 Factorio 和 ModpackBuilder，并先复制保存当前文件：

1. 恢复模组包：将所需 ZIP 备份复制到 mods 目录，去掉文件名末尾的 `.<时间戳>.bak`，恢复为 `<内部名>_<版本>.zip`。
2. 恢复模组列表：将操作前的 `mod-list.json.<时间戳>.bak` 复制到 mods 目录并重命名为 `mod-list.json`。
3. 若只需恢复 ZIP 但保留当前模组列表，请确认当前 `mod-list.json` 中已有对应条目；否则同时恢复相匹配的列表备份。

恢复整个 `mod-list.json` 会回退该文件在备份之后的所有改动，操作前务必核对时间戳。

## 限制

- 仅支持 Windows 和 Factorio 2.1。
- 仅扫描 mods 目录顶层，不读取 `mod-settings.dat`，不执行模组代码。
- 静态检查不能发现数据阶段或运行时的所有冲突。
- 缺失必需依赖、依赖未纳入合集等情况通常只警告，仍允许生成。
- 不自动安装、删除或禁用成员模组。
- 游戏正在运行时工具会退出；写入前的再次检测失败也会中止操作。

## 卸载

关闭程序后删除 exe、`config.json` 和不再需要的 `backups/` 即可。卸载工具不会自动删除已生成的模组包；请先在工具中删除，或在关闭 Factorio 后手工移除对应 ZIP 和 `mod-list.json` 条目。

## 从源码构建与验证

需要 Rust 工具链（开发基线为 rustc / cargo 1.99.1）：

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
.\target\release\ModpackBuilder.exe --help
```

发布构建位于 `target/release/ModpackBuilder.exe`。

## 许可证

本项目采用 MIT 许可证。
