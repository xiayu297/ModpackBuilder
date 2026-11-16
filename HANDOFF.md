# ModpackBuilder 开发交接文档

- 项目路径：`D:\Code\Temp\ModpackBuilder`
- 最后更新：2026-11-16
- 工具链：rustc / cargo 1.99.1
- 当前阶段：核心功能、TUI、设置、依赖修复、显示标题前缀选项、文档和发布构建已完成；28 个单元测试与 3 个 CLI 集成测试通过，Clippy 无警告；此前的人工 TUI/设置验收及 Factorio 2.1 实际加载验收均成功。

## 1. 项目目标与不可违反的约束

为 Factorio **2.1** 制作便携式 Windows 命令行工具 **ModpackBuilder**（中文显示名 **模组包创建器**）：扫描已安装模组，创建只声明成员依赖的模组包并注册到 `mod-list.json`，支持后续编辑和删除。

1. 仅支持英文和简体中文；首次启动选择语言并保存到 exe 同目录 `config.json`。
2. 只发布 Windows 便携版单文件 exe，不写注册表、不提供安装器。
3. 运行时只允许写入 Factorio mods 目录，以及 exe 所在目录的 `config.json` 和 `backups/`。
4. 检测到 Factorio 正在运行时提示并退出，不扫描、不写入；每次写操作前再次检测。
5. `mod-list.json` 只做字节保留追加/删除，绝不整文件重新序列化。
6. 新注册条目为禁用状态；不删除、不禁用任何成员模组。
7. 生成物不包含成员模组本体，不生成 `.lua` 或 `thumbnail.png`。
8. 每次启动和每次成功增删改后重新扫描，不缓存。
9. 只做静态兼容检查，不执行模组代码，不读取 `mod-settings.dat` 或 `factorio-current.log`。
10. 备份默认保留 7 天，直接放在 exe 同目录 `backups/`；`0` 表示不自动清理。

## 2. 当前完成状态

### 功能

- 扫描 mods 目录顶层 ZIP 与文件夹模组；ZIP 内 `info.json` 是权威元数据。
- 同内部名多版本取最高版本并警告；内置/DLC 模组列出但不可选。
- 英/简中双语 TUI：首次语言选择、主菜单、创建、编辑、删除、成员过滤多选、兼容性问题展示。
- 生成格式符合 ADR 0003，带 `module-set-builder.json` marker；不生成 Lua 或缩略图。
- `mod-list.json` 保留 BOM、CRLF、缩进、空行和字段顺序；写前重解析校验并原子替换。
- 创建、更新、删除带备份和回滚处理；更新保留已有条目的启用状态，缺失条目补为禁用。
- 创建要求全新内部名；重名必须走编辑，不覆盖、不自动递增版本。
- 目标文件冲突会在流程内提示并返回主菜单，不修改原文件。
- 主菜单、成员多选、编辑/删除选择的操作帮助已按语言显示，并明确 Esc 退出/返回；首次语言选择使用双语帮助。
- 创建、更新、删除的落地操作失败会在流程内显示完整错误并返回主菜单，不再直接退出整个程序。
- 终端故障、扫描/重载失败及游戏运行检测等致命错误仍会退出。
- CLI：`--mods-dir`、`--lang`、`--backup-days`、`--help`、`--version`。
- 主菜单“设置”仅提供界面语言和备份保留天数；保存成功后才应用，保存失败不改变当前状态。
- 未使用 CLI 覆盖时语言立即切换、保留天数立即生效；显式 `--lang` / `--backup-days` 在本次运行中始终优先，设置保存的新值供以后不带对应参数启动时使用。
- 保留天数直接输入非负整数，0 表示不自动清理；修改设置时不立即清理备份，仍只在启动及成功增删改后清理。
- `--lang` 与 `--backup-days` 参数本身仅临时覆盖，不改写已保存配置。
- 依赖约束版本接受 `x.y` 与 `x.y.z`，两段按 `x.y.0` 比较；模组包自身版本仍严格要求三段。
- 依赖名称允许内部空格；`base >= 2.1` 正常解析并由内置依赖检查跳过，`! Squeak Through` 正常参与冲突检查。
- 两个冲突成员同时选中会阻止生成；冲突目标已安装但未选中只警告并显示启用状态，未安装则不提示；无法解析的声明只警告。
- 创建和编辑输入显示标题后可单次选择是否添加 `⚙️` 前缀；不保存为设置，不重复添加，选择“否”会移除已有前缀，内部名和加载顺序不变。

### 收尾工作

- `README.md` 已包含构建、使用、CLI、备份、恢复、限制和卸载说明。
- `LICENSE` 已添加 MIT 许可证。
- `.gitignore` 已忽略 `/target`、根目录运行期 `config.json` 和 `backups/`。
- 已移除确认未使用的 `pack::write_pack`、`Marker::member_names`、`InstalledMod::enabled_state` 等遗留 helper。
- 核心 `pack::validate_spec` 与 UI 均拒绝带控制字符的作者名。
- ADR 0003/0005、术语表与实现已统一：双语描述、扁平备份目录、3~50 字符内部名、不读取游戏日志。

### 自动验证结果（2026-11-16）

- `cargo fmt --check`：通过。
- `cargo test`：**28 个单元测试 + 3 个 CLI 集成测试，全部通过**。
- `cargo clippy --all-targets -- -D warnings`：通过，无警告。
- `cargo build --release`：通过。
- `target\release\ModpackBuilder.exe --help`：通过，选项输出正确。
- 隔离样本启动扫描：共 8 个模组（5 个内置、2 个兼容、1 个不兼容），成功渲染主菜单；未自动注入按键，等待输入的进程已停止。
- 用户已完成人工 TUI 创建/编辑/删除与设置验收，未发现阻塞问题。
- 用户已在 Factorio 2.1 中实际启用新生成的模组包，游戏解析和加载成功。
- 发布程序：`target\release\ModpackBuilder.exe`；`target\smoke\ModpackBuilder.exe` 已同步，二者 SHA-256 均为 `cfa295637224ca5ba22d134a7ef2d99dc1d309bd9819d7814de3ec44d8aeee3c`。

## 3. 代码与文档入口

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | clap CLI 入口，定位 exe 所在目录并调用 UI。 |
| `src/ui.rs` | 全部交互流程、显示、输入校验、业务错误恢复。 |
| `src/types.rs` | 版本、依赖、模组、marker、成员和 PackSpec 类型。 |
| `src/modscan.rs` | ZIP/文件夹扫描、重复版本筛选、marker 识别。 |
| `src/compat.rs` | 静态依赖检查与硬/软问题分类。 |
| `src/pack.rs` | 规格校验、ZIP 内容、locale、changelog、marker。 |
| `src/modlist.rs` | `mod-list.json` 字节保留追加/删除及原子写。 |
| `src/backup.rs` | 备份命名和按保留天数清理。 |
| `src/factorio.rs` | mods 路径解析和 Factorio 进程检测。 |
| `src/config.rs` | exe 同目录 `config.json` 读写。 |
| `src/operations.rs` | 创建、更新、删除、备份和回滚。 |
| `tests/cli.rs` | 非交互 CLI 集成测试。 |

设计文档：

- `docs/glossary.md`
- `docs/adr/0001-target-platform-and-portability.md`
- `docs/adr/0002-scan-and-compatibility.md`
- `docs/adr/0003-modpack-format.md`
- `docs/adr/0004-byte-preserving-mod-list.md`
- `docs/adr/0005-backups-edit-and-delete.md`
- `docs/adr/0006-ui-and-language.md`

## 4. 人工验收记录与复测环境

用户已完成 TUI 创建/编辑/删除、设置及 Factorio 2.1 实际启用验收，结果成功。保留以下隔离环境供后续复测，不会触碰真实游戏数据：

- `ModpackBuilder.exe`：本次 release 构建的副本。
- `mods\mod-list.json`：带空行和不同条目格式的测试文件。
- `alpha-mod_1.0.0.zip`：Factorio 2.1，中文标题，已启用。
- `beta-mod_2.0.0.zip`：Factorio 2.1，禁用，可选依赖 alpha。
- `legacy-mod_1.0.0.zip`：Factorio 2.0，应扫描为不兼容且不可选。

在 PowerShell 中运行：

```powershell
cd D:\Code\Temp\ModpackBuilder\target\smoke
.\ModpackBuilder.exe --mods-dir .\mods --backup-days 7
```

不要传入 `--lang`，以便验证首次语言选择和 `config.json` 保存。

人工检查：

1. 首次选择“简体中文”，确认界面中文显示且 exe 同目录生成 `config.json`。
2. 确认扫描摘要包含两个兼容成员、一个不兼容模组；`legacy-mod` 不可选。
3. 创建：测试成员过滤、多选和取消，输入中文标题/作者，创建 `1.0.0` 模组包。
4. 确认新 ZIP 生成、`mod-list.json` 只追加禁用条目、原有格式保留、成员 ZIP 未改动、`backups/` 有列表备份。
5. 编辑：修改标题、作者、成员和版本到 `1.1.0`；确认旧 ZIP 移出、已有启用状态保留、ZIP 有备份。
6. 删除：先验证默认“否”，再确认删除；确认只删除模组包 ZIP 和其条目，成员仍在，ZIP 与列表备份保留。
7. 测试 Esc 取消、空成员、重复内部名、目标冲突和警告确认行为。
8. 特别观察中文输入、MultiSelect 过滤、终端宽度及所有确认提示。
9. 不带 CLI 覆盖进入“设置”：切换英文/中文应立即生效；修改保留天数应保存到 `config.json`，Esc 应从子页返回设置、从设置返回主菜单，且不会立即清理备份。
10. 使用 `--lang en --backup-days 30` 启动后进入设置：对应项应显示 CLI 覆盖；保存新的默认语言和天数后，本次仍使用英文和 30 天。
11. 不带对应 CLI 参数重新启动：应读取设置中保存的新默认值；检查无效天数、0、最大值和保存失败提示。
12. 创建和编辑时，在标题输入后检查 `⚙️` 前缀确认：选择“是”应显示为 `⚙️ 标题`，重复编辑不叠加；选择“否”应移除已有前缀。此新增交互尚未人工复测。

已知限制：当前环境的 ConPTY 自动注键不可靠。不要重新引入 expectrl 冒烟测试；后续改动仍需人工复测上述关键流程并重跑全部自动验证。

## 5. 构建与验证命令

```powershell
cd D:\Code\Temp\ModpackBuilder
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
.\target\release\ModpackBuilder.exe --help
```

## 6. Git 状态与后续

- 仓库仍无提交，项目文件处于未跟踪状态；本轮未提交、未推送。
- 用户未明确要求前不要提交或推送。
- 人工验收已通过，可直接交付 `target\release\ModpackBuilder.exe`；若需要其他交付位置或打包形式，再由用户决定。
