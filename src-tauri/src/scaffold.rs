use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Emitter};

const DEFAULT_MONOLITH_PORT: u16 = 48080;
const DEFAULT_GATEWAY_PORT: u16 = 48080;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPayload {
    answers: ScaffoldAnswers,
    workspace: Option<String>,
    mirror: Option<String>,
    #[serde(default)]
    url_overrides: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScaffoldAnswers {
    project_name: String,
    display_name: String,
    output_dir: String,
    backend: String,
    jdk_version: String,
    group_id: String,
    artifact_id: String,
    version: String,
    base_package: String,
    modules: Vec<String>,
    frontends: Vec<String>,
    monolith_port: Option<u16>,
    gateway_port: Option<u16>,
    #[serde(default)]
    microservice_ports: HashMap<String, Vec<u16>>,
    super_admin_username: String,
    super_admin_password: String,
    pull_existing: bool,
    force: Option<bool>,
    tenant_enabled: bool,
    vben_variant: Option<String>,
}

#[derive(Clone, Copy)]
struct TemplateSource {
    name: &'static str,
    kind: &'static str,
    github: &'static str,
    gitee: &'static str,
}

#[derive(Clone, Copy)]
struct FrontendSource {
    id: &'static str,
    template: &'static str,
    role_suffix: &'static str,
}

struct RuntimePaths {
    home_dir: PathBuf,
    workspace: PathBuf,
    cache_dir: PathBuf,
}

struct StagedOutputDir {
    path: PathBuf,
    committed: bool,
}

impl StagedOutputDir {
    fn new(target: &Path) -> Result<Self, String> {
        let parent = target
            .parent()
            .ok_or_else(|| format!("输出目录缺少父目录: {}", target.display()))?;
        fs::create_dir_all(parent).map_err(|e| format!("创建输出父目录失败: {e}"))?;
        let path = unique_sibling_path(target, "staging")?;
        fs::create_dir(&path).map_err(|e| format!("创建生成暂存目录失败: {e}"))?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self, target: &Path) -> Result<Option<String>, String> {
        let backup = unique_sibling_path(target, "backup")?;
        let had_existing = target.exists();
        if had_existing {
            fs::rename(target, &backup).map_err(|e| format!("暂存原输出目录失败: {e}"))?;
        }
        if let Err(error) = fs::rename(&self.path, target) {
            let restore_error = if had_existing {
                fs::rename(&backup, target).err()
            } else {
                None
            };
            return Err(match restore_error {
                Some(restore) => {
                    format!("提交生成目录失败: {error}；恢复原输出目录也失败: {restore}")
                }
                None => format!("提交生成目录失败: {error}"),
            });
        }
        self.committed = true;
        if had_existing {
            if let Err(error) = fs::remove_dir_all(&backup) {
                return Ok(Some(format!(
                    "新项目已生成，但旧目录备份清理失败，请手动删除 {}: {error}",
                    backup.display()
                )));
            }
        }
        Ok(None)
    }
}

impl Drop for StagedOutputDir {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[tauri::command]
pub async fn load_meta(workspace: Option<String>) -> Result<serde_json::Value, String> {
    let paths = runtime_paths(workspace)?;
    fs::create_dir_all(&paths.cache_dir).map_err(|e| format!("创建模板缓存目录失败: {e}"))?;

    Ok(json!({
        "workspace": path_string(&paths.workspace),
        "cacheDir": path_string(&paths.cache_dir),
        "homeDir": path_string(&paths.home_dir),
        "defaultMirror": "gitee",
        "defaultMonolithPort": DEFAULT_MONOLITH_PORT,
        "defaultGatewayPort": DEFAULT_GATEWAY_PORT,
        "modules": module_meta(),
        "frontends": frontend_meta(),
        "templates": template_meta(&paths),
    }))
}

#[tauri::command]
pub async fn run_scaffold(app: AppHandle, payload: RunPayload) -> Result<i32, String> {
    let paths = runtime_paths(payload.workspace.clone())?;
    fs::create_dir_all(&paths.cache_dir).map_err(|e| format!("创建模板缓存目录失败: {e}"))?;

    let answers = &payload.answers;
    let output_dir = PathBuf::from(answers.output_dir.trim());
    if output_dir.as_os_str().is_empty() {
        return Err("请选择输出目录".to_string());
    }
    if !output_dir.is_absolute() {
        return Err("输出目录必须是绝对路径".to_string());
    }
    if output_dir.exists() {
        if answers.force != Some(true) {
            return Err("输出目录已存在，请确认强制覆盖后再生成".to_string());
        }
        if !output_dir.is_dir() {
            return Err("输出路径已存在，但不是目录".to_string());
        }
        guard_removable_output_dir(&output_dir)?;
    }
    let mut staged_output = StagedOutputDir::new(&output_dir)?;
    let generation_dir = staged_output.path().to_path_buf();

    let mut selected_templates = Vec::new();
    selected_templates.push(if answers.backend == "microservice" {
        "yudao-cloud"
    } else {
        "ruoyi-vue-pro"
    });
    for frontend_id in &answers.frontends {
        if let Some(frontend) = frontend_source(frontend_id) {
            selected_templates.push(frontend.template);
        }
    }

    let total = selected_templates.len() + 3;
    emit_phase(&app, 1, total, "准备输出目录");
    emit_info(&app, &format!("输出目录: {}", output_dir.display()));

    let mirror = payload.mirror.as_deref().unwrap_or("gitee");
    let client = reqwest::Client::builder()
        .user_agent(concat!("yudao-scaffold-ui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建下载客户端失败: {e}"))?;

    let mut phase_index = 2;
    let backend_name = selected_templates[0];
    let backend_template =
        template_source(backend_name).ok_or_else(|| format!("未知后端模板: {backend_name}"))?;
    emit_phase(
        &app,
        phase_index,
        total,
        &format!("准备后端模板 {backend_name}"),
    );
    let backend_src = prepare_template(
        &client,
        backend_template,
        Some(backend_template_branch(&answers.jdk_version)?),
        &paths,
        mirror,
        &payload.url_overrides,
        answers.pull_existing,
        &app,
    )
    .await?;
    let backend_dst = generation_dir.join("backend");
    copy_dir_contents(&backend_src, &backend_dst).map_err(|e| format!("复制后端模板失败: {e}"))?;
    prune_backend_modules(&backend_dst, &answers.modules)?;
    customize_backend_tree(&backend_dst, answers)?;
    emit_ok(&app, "后端模板已写入 backend/");

    for frontend_id in &answers.frontends {
        let Some(frontend) = frontend_source(frontend_id) else {
            emit_warn(&app, &format!("跳过未知前端: {frontend_id}"));
            continue;
        };
        phase_index += 1;
        let template = template_source(frontend.template)
            .ok_or_else(|| format!("未知前端模板: {}", frontend.template))?;
        emit_phase(
            &app,
            phase_index,
            total,
            &format!("准备前端模板 {}", frontend.template),
        );
        let src = prepare_template(
            &client,
            template,
            None,
            &paths,
            mirror,
            &payload.url_overrides,
            answers.pull_existing,
            &app,
        )
        .await?;
        let dst = generation_dir.join("frontend").join(frontend.role_suffix);
        copy_dir_contents(&src, &dst).map_err(|e| format!("复制前端模板失败: {e}"))?;
        customize_frontend_tree(&dst, frontend_id, answers)?;
        emit_ok(
            &app,
            &format!(
                "前端模板 {} 已写入 frontend/{}/",
                frontend.template, frontend.role_suffix
            ),
        );
    }

    emit_phase(&app, total - 1, total, "写入脚手架说明");
    write_scaffold_manifest(&generation_dir, &payload)?;

    if let Some(warning) = staged_output.commit(&output_dir)? {
        emit_warn(&app, &warning);
    }

    emit_phase(&app, total, total, "生成完成");
    emit_done(&app, &path_string(&output_dir));
    Ok(0)
}

async fn prepare_template(
    client: &reqwest::Client,
    template: TemplateSource,
    branch: Option<&str>,
    paths: &RuntimePaths,
    mirror: &str,
    overrides: &HashMap<String, String>,
    use_cache: bool,
    app: &AppHandle,
) -> Result<PathBuf, String> {
    let local_path = paths.workspace.join(template.name);
    if directory_has_entries(&local_path) {
        emit_info(app, &format!("使用本地模板: {}", local_path.display()));
        return Ok(local_path);
    }

    let cache_path = paths
        .cache_dir
        .join(template_cache_dir_name(template.name, branch));
    if use_cache && directory_has_entries(&cache_path) {
        emit_info(app, &format!("使用缓存模板: {}", cache_path.display()));
        return Ok(cache_path);
    }
    if cache_path.exists() {
        emit_warn(
            app,
            &format!("忽略不完整的模板缓存: {}", cache_path.display()),
        );
    }

    let source_urls = if let Some(url_override) = overrides.get(template.name) {
        vec![url_override.as_str()]
    } else if mirror == "github" {
        vec![template.github, template.gitee]
    } else {
        vec![template.gitee, template.github]
    };
    let candidates = source_urls
        .iter()
        .flat_map(|url| archive_url_candidates(url, branch))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(format!(
            "无法从模板地址生成下载链接: {}",
            source_urls.join(", ")
        ));
    }

    emit_info(app, &format!("下载模板 {} ...", template.name));
    download_and_extract(client, &candidates, &cache_path).await?;
    emit_ok(app, &format!("模板 {} 已缓存", template.name));
    Ok(cache_path)
}

async fn download_and_extract(
    client: &reqwest::Client,
    candidates: &[String],
    destination: &Path,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| format!("无效缓存目录: {}", destination.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("创建缓存父目录失败: {e}"))?;

    let zip_path = parent.join(format!(
        ".{}.zip",
        destination
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("template")
    ));
    let staging_path = parent.join(format!(
        ".{}.extracting",
        destination
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("template")
    ));

    let mut errors = Vec::new();
    for url in candidates {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let content_type = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("未知")
                    .to_string();
                let bytes = match resp.bytes().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        errors.push(format!("{url} 读取响应失败: {e}"));
                        continue;
                    }
                };
                if !has_zip_signature(&bytes) {
                    errors.push(format!(
                        "{url} 返回的不是 ZIP（Content-Type: {content_type}，{} 字节）",
                        bytes.len()
                    ));
                    continue;
                }
                tokio::fs::write(&zip_path, &bytes)
                    .await
                    .map_err(|e| format!("写入模板压缩包失败: {e}"))?;
                let zip_path_clone = zip_path.clone();
                let staging_path_clone = staging_path.clone();
                let extraction = tokio::task::spawn_blocking(move || {
                    extract_zip_strip_root(&zip_path_clone, &staging_path_clone)
                })
                .await;
                match extraction {
                    Ok(Ok(())) => {
                        if destination.exists() {
                            fs::remove_dir_all(destination)
                                .map_err(|e| format!("清理旧缓存失败: {e}"))?;
                        }
                        fs::rename(&staging_path, destination)
                            .map_err(|e| format!("提交模板缓存失败: {e}"))?;
                        let _ = fs::remove_file(&zip_path);
                        return Ok(());
                    }
                    Ok(Err(e)) => {
                        errors.push(format!("{url} 解压失败: {e}"));
                    }
                    Err(e) => {
                        errors.push(format!("{url} 解压任务失败: {e}"));
                    }
                }
                let _ = fs::remove_dir_all(&staging_path);
                let _ = fs::remove_file(&zip_path);
            }
            Ok(resp) => {
                errors.push(format!("{url} 返回 HTTP {}", resp.status()));
            }
            Err(e) => {
                errors.push(format!("{url} 下载失败: {e}"));
            }
        }
    }

    let _ = fs::remove_dir_all(&staging_path);
    let _ = fs::remove_file(&zip_path);
    Err(format!("模板下载失败: {}", errors.join("；")))
}

fn has_zip_signature(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([b'P', b'K', 3, 4] | [b'P', b'K', 5, 6] | [b'P', b'K', 7, 8])
    )
}

fn directory_has_entries(path: &Path) -> bool {
    path.is_dir()
        && fs::read_dir(path)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false)
}

fn backend_template_branch(jdk_version: &str) -> Result<&'static str, String> {
    match jdk_version {
        "8" => Ok("master"),
        "17" => Ok("master-jdk17"),
        other => Err(format!("不支持的 JDK 版本: {other}")),
    }
}

fn template_cache_dir_name(template_name: &str, branch: Option<&str>) -> String {
    match branch {
        Some("master") | None => template_name.to_string(),
        Some(branch) => format!(
            "{template_name}-{}",
            branch.strip_prefix("master-").unwrap_or(branch)
        ),
    }
}

fn extract_zip_strip_root(zip_path: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        fs::remove_dir_all(destination).map_err(|e| format!("清理旧缓存失败: {e}"))?;
    }
    fs::create_dir_all(destination).map_err(|e| format!("创建解压目录失败: {e}"))?;

    let file = File::open(zip_path).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("读取 zip 失败: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取 zip 条目失败: {e}"))?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let relative = strip_first_component(&enclosed);
        if relative.as_os_str().is_empty() {
            continue;
        }
        let out_path = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| format!("创建目录失败: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建文件目录失败: {e}"))?;
            }
            let mut out_file = File::create(&out_path).map_err(|e| format!("创建文件失败: {e}"))?;
            io::copy(&mut entry, &mut out_file).map_err(|e| format!("写入文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn copy_dir_contents(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("source not found: {}", source.display()),
        ));
    }
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;
    copy_dir_inner(source, destination)
}

fn copy_dir_inner(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let src = entry.path();
        let dst = destination.join(entry.file_name());
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | ".idea" | ".vscode"
        ) {
            continue;
        }
        if src.is_dir() {
            fs::create_dir_all(&dst)?;
            copy_dir_inner(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

const SELECTABLE_MODULES: &[&str] = &[
    "system", "infra", "member", "bpm", "pay", "mp", "mall", "crm", "erp", "iot", "mes", "report",
    "ai",
];

fn prune_backend_modules(root: &Path, selected_modules: &[String]) -> Result<(), String> {
    let selected = selected_modules
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    for required in ["system", "infra"] {
        if !selected.contains(required) {
            return Err(format!("缺少必选业务模块: {required}"));
        }
    }
    for module in &selected {
        if !SELECTABLE_MODULES.contains(module) {
            return Err(format!("未知业务模块: {module}"));
        }
        let module_dir = root.join(format!("yudao-module-{module}"));
        if !module_dir.is_dir() {
            return Err(format!(
                "模板缺少所选业务模块目录: {}",
                module_dir.display()
            ));
        }
    }

    for entry in fs::read_dir(root).map_err(|e| format!("读取后端目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取后端目录项失败: {e}"))?;
        if !entry
            .file_type()
            .map_err(|e| format!("读取后端目录项类型失败: {e}"))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(module) = name.strip_prefix("yudao-module-") else {
            continue;
        };
        if !selected.contains(module) {
            fs::remove_dir_all(entry.path())
                .map_err(|e| format!("裁剪未选模块 {name} 失败: {e}"))?;
        }
    }

    activate_root_modules(&root.join("pom.xml"), &selected)?;
    let server_pom = root.join("yudao-server").join("pom.xml");
    if server_pom.is_file() {
        activate_server_dependencies(&server_pom, &selected)?;
    }
    Ok(())
}

fn activate_root_modules(pom_path: &Path, selected: &HashSet<&str>) -> Result<(), String> {
    let text = fs::read_to_string(pom_path)
        .map_err(|e| format!("读取根 Maven POM 失败 {}: {e}", pom_path.display()))?;
    let mut changed = false;
    let lines = text
        .lines()
        .map(|line| {
            let should_activate = selected
                .iter()
                .any(|module| line.contains(&format!("<module>yudao-module-{module}</module>")));
            if should_activate && line.contains("<!--") {
                changed = true;
                line.replace("<!--", "").replace("-->", "")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    if changed {
        write_lines_preserving_final_newline(pom_path, &text, lines, "写入根 Maven POM")?;
    }
    Ok(())
}

fn activate_server_dependencies(pom_path: &Path, selected: &HashSet<&str>) -> Result<(), String> {
    let artifacts = selected
        .iter()
        .flat_map(|module| module_server_artifacts(module))
        .collect::<HashSet<_>>();
    let text = fs::read_to_string(pom_path)
        .map_err(|e| format!("读取 Server Maven POM 失败 {}: {e}", pom_path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let mut changed = false;
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].contains("<dependency>") {
            index += 1;
            continue;
        }
        let end = (index..lines.len())
            .find(|candidate| lines[*candidate].contains("</dependency>"))
            .unwrap_or(index);
        let should_activate = lines[index..=end].iter().any(|line| {
            artifacts
                .iter()
                .any(|artifact| line.contains(&format!("<artifactId>{artifact}</artifactId>")))
        });
        if should_activate {
            for line in &mut lines[index..=end] {
                if line.contains("<!--") || line.contains("-->") {
                    *line = line.replace("<!--", "").replace("-->", "");
                    changed = true;
                }
            }
        }
        index = end + 1;
    }
    if changed {
        write_lines_preserving_final_newline(pom_path, &text, lines, "写入 Server Maven POM")?;
    }
    Ok(())
}

fn module_server_artifacts(module: &str) -> &'static [&'static str] {
    match module {
        "mall" => &[
            "yudao-module-product",
            "yudao-module-promotion",
            "yudao-module-trade",
            "yudao-module-statistics",
        ],
        "iot" => &["yudao-module-iot-biz"],
        "system" => &["yudao-module-system"],
        "infra" => &["yudao-module-infra"],
        "member" => &["yudao-module-member"],
        "bpm" => &["yudao-module-bpm"],
        "pay" => &["yudao-module-pay"],
        "mp" => &["yudao-module-mp"],
        "crm" => &["yudao-module-crm"],
        "erp" => &["yudao-module-erp"],
        "mes" => &["yudao-module-mes"],
        "report" => &["yudao-module-report"],
        "ai" => &["yudao-module-ai"],
        _ => &[],
    }
}

fn write_lines_preserving_final_newline(
    path: &Path,
    original: &str,
    lines: Vec<String>,
    action: &str,
) -> Result<(), String> {
    let mut updated = lines.join("\n");
    if original.ends_with('\n') {
        updated.push('\n');
    }
    fs::write(path, updated).map_err(|e| format!("{action}失败 {}: {e}", path.display()))
}

fn customize_backend_tree(root: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    let slash_package = answers.base_package.replace('.', "/");
    let backslash_package = answers.base_package.replace('.', "\\");
    let artifact_tag = format!("<artifactId>{}</artifactId>", answers.artifact_id);
    let replacements = [
        ("cn.iocoder.yudao", answers.base_package.as_str()),
        ("cn/iocoder/yudao", slash_package.as_str()),
        ("cn\\iocoder\\yudao", backslash_package.as_str()),
        ("cn.iocoder.boot", answers.group_id.as_str()),
        ("<artifactId>yudao</artifactId>", artifact_tag.as_str()),
        ("ruoyi-vue-pro", answers.project_name.as_str()),
        ("yudao-cloud", answers.project_name.as_str()),
    ];
    rewrite_text_files(root, &replacements)?;
    set_xml_tag_value(&root.join("pom.xml"), "revision", &answers.version)?;
    relocate_java_packages(root, &answers.base_package)?;
    configure_backend_settings(root, answers)?;
    Ok(())
}

fn customize_frontend_tree(
    root: &Path,
    frontend_id: &str,
    answers: &ScaffoldAnswers,
) -> Result<(), String> {
    rewrite_text_files(
        root,
        &[
            ("ruoyi-vue-pro", answers.project_name.as_str()),
            ("yudao-cloud", answers.project_name.as_str()),
        ],
    )?;
    if frontend_id == "admin-vben" {
        prune_vben_variants(
            &root.join("apps"),
            answers.vben_variant.as_deref().unwrap_or("antd"),
        )?;
    }
    Ok(())
}

fn prune_vben_variants(apps_dir: &Path, selected_variant: &str) -> Result<(), String> {
    const VARIANTS: &[&str] = &["antd", "antdv-next", "ele", "naive", "tdesign"];
    if !VARIANTS.contains(&selected_variant) {
        return Err(format!("未知 Vben UI 变体: {selected_variant}"));
    }
    let selected_dir = apps_dir.join(format!("web-{selected_variant}"));
    if !selected_dir.is_dir() {
        return Err(format!("Vben 模板缺少所选变体: {}", selected_dir.display()));
    }
    for variant in VARIANTS {
        if *variant == selected_variant {
            continue;
        }
        let path = apps_dir.join(format!("web-{variant}"));
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("裁剪 Vben 变体 {} 失败: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn set_xml_tag_value(path: &Path, tag: &str, value: &str) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取 XML 文件失败 {}: {e}", path.display()))?;
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text
        .find(&open)
        .ok_or_else(|| format!("{} 缺少 {open}", path.display()))?
        + open.len();
    let end = text[start..]
        .find(&close)
        .map(|offset| start + offset)
        .ok_or_else(|| format!("{} 缺少 {close}", path.display()))?;
    let mut updated = text;
    updated.replace_range(start..end, value);
    fs::write(path, updated).map_err(|e| format!("写入 XML 文件失败 {}: {e}", path.display()))
}

fn configure_backend_settings(root: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    if answers.backend == "monolith" {
        let port = answers
            .monolith_port
            .ok_or_else(|| "单体项目缺少服务端口".to_string())?;
        for profile in ["application-local.yaml", "application-dev.yaml"] {
            set_top_level_server_port(
                &root
                    .join("yudao-server")
                    .join("src")
                    .join("main")
                    .join("resources")
                    .join(profile),
                port,
            )?;
        }
        set_tenant_enabled(
            &root
                .join("yudao-server")
                .join("src")
                .join("main")
                .join("resources")
                .join("application.yaml"),
            answers.tenant_enabled,
        )?;
    } else if answers.backend == "microservice" {
        configure_microservice_settings(root, answers)?;
    } else {
        return Err(format!("未知后端类型: {}", answers.backend));
    }
    configure_super_admin(
        &root.join("sql"),
        &answers.super_admin_username,
        &answers.super_admin_password,
    )
}

fn configure_microservice_settings(root: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    let gateway_port = answers
        .gateway_port
        .ok_or_else(|| "微服务项目缺少网关端口".to_string())?;
    let mut used_ports = HashMap::from([(gateway_port, "gateway".to_string())]);
    configure_microservice_port(
        &root
            .join("yudao-gateway")
            .join("src")
            .join("main")
            .join("resources"),
        gateway_port,
    )?;

    for module in &answers.modules {
        let resource_dirs = microservice_resource_dirs(root, module);
        let ports = answers
            .microservice_ports
            .get(module)
            .cloned()
            .unwrap_or_else(|| default_microservice_ports(module).to_vec());
        if resource_dirs.len() != ports.len() {
            return Err(format!(
                "模块 {module} 需要 {} 个端口，但收到 {} 个",
                resource_dirs.len(),
                ports.len()
            ));
        }
        for (index, (resources, port)) in resource_dirs.iter().zip(ports).enumerate() {
            let owner = if resource_dirs.len() == 1 {
                module.clone()
            } else {
                format!("{module}[{index}]")
            };
            if let Some(existing) = used_ports.insert(port, owner.clone()) {
                return Err(format!("端口 {port} 同时分配给 {existing} 和 {owner}"));
            }
            configure_microservice_port(resources, port)?;
            let _ = set_tenant_enabled_if_present(
                &resources.join("application.yaml"),
                answers.tenant_enabled,
            )?;
        }
    }
    Ok(())
}

fn microservice_resource_dirs(root: &Path, module: &str) -> Vec<PathBuf> {
    let servers: &[&str] = match module {
        "mall" => &[
            "yudao-module-product-server",
            "yudao-module-trade-server",
            "yudao-module-promotion-server",
            "yudao-module-statistics-server",
        ],
        "iot" => &["yudao-module-iot-server"],
        "system" => &["yudao-module-system-server"],
        "infra" => &["yudao-module-infra-server"],
        "member" => &["yudao-module-member-server"],
        "bpm" => &["yudao-module-bpm-server"],
        "pay" => &["yudao-module-pay-server"],
        "mp" => &["yudao-module-mp-server"],
        "crm" => &["yudao-module-crm-server"],
        "erp" => &["yudao-module-erp-server"],
        "mes" => &["yudao-module-mes-server"],
        "report" => &["yudao-module-report-server"],
        "ai" => &["yudao-module-ai-server"],
        _ => &[],
    };
    servers
        .iter()
        .map(|server| {
            root.join(format!("yudao-module-{module}"))
                .join(server)
                .join("src")
                .join("main")
                .join("resources")
        })
        .collect()
}

fn default_microservice_ports(module: &str) -> &'static [u16] {
    match module {
        "system" => &[48081],
        "infra" => &[48082],
        "member" => &[48087],
        "bpm" => &[48083],
        "pay" => &[48085],
        "mp" => &[48086],
        "mall" => &[48100, 48102, 48101, 48103],
        "crm" => &[48089],
        "erp" => &[48088],
        "iot" => &[48091],
        "mes" => &[48092],
        "report" => &[48084],
        "ai" => &[48090],
        _ => &[],
    }
}

fn configure_microservice_port(resources: &Path, port: u16) -> Result<(), String> {
    set_top_level_server_port(&resources.join("application.yaml"), port)?;
    let server_dir = resources
        .ancestors()
        .nth(3)
        .ok_or_else(|| format!("无法定位微服务目录: {}", resources.display()))?;
    set_docker_expose(&server_dir.join("Dockerfile"), port)
}

fn set_docker_expose(path: &Path, port: u16) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取 Dockerfile 失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let expose = lines
        .iter()
        .position(|line| line.trim_start().starts_with("EXPOSE "))
        .ok_or_else(|| format!("{} 缺少 EXPOSE 配置", path.display()))?;
    lines[expose] = format!("EXPOSE {port}");
    write_lines_preserving_final_newline(path, &text, lines, "写入 Docker 端口")
}

fn set_top_level_server_port(path: &Path, port: u16) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取服务配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let server_index = lines
        .iter()
        .position(|line| line.trim() == "server:" && !line.starts_with(char::is_whitespace))
        .ok_or_else(|| format!("{} 缺少顶层 server 配置", path.display()))?;
    let port_index = ((server_index + 1)..lines.len())
        .take_while(|index| {
            let line = &lines[*index];
            line.trim().is_empty() || line.starts_with(char::is_whitespace)
        })
        .find(|index| lines[*index].trim_start().starts_with("port:"))
        .ok_or_else(|| format!("{} 缺少 server.port 配置", path.display()))?;
    let indent = lines[port_index]
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect::<String>();
    lines[port_index] = format!("{indent}port: {port}");
    let old_localhost = "localhost:48080";
    let old_loopback = "127.0.0.1:48080";
    for line in &mut lines {
        *line = line
            .replace(old_localhost, &format!("localhost:{port}"))
            .replace(old_loopback, &format!("127.0.0.1:{port}"));
    }
    write_lines_preserving_final_newline(path, &text, lines, "写入服务端口")
}

fn set_tenant_enabled(path: &Path, enabled: bool) -> Result<(), String> {
    if set_tenant_enabled_if_present(path, enabled)? {
        Ok(())
    } else {
        Err(format!("{} 缺少 yudao.tenant 配置", path.display()))
    }
}

fn set_tenant_enabled_if_present(path: &Path, enabled: bool) -> Result<bool, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取租户配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let Some(yudao_index) = lines
        .iter()
        .position(|line| line.trim() == "yudao:" && !line.starts_with(char::is_whitespace))
    else {
        return Ok(false);
    };
    let Some(tenant_index) = ((yudao_index + 1)..lines.len())
        .take_while(|index| {
            let line = &lines[*index];
            line.trim().is_empty()
                || line.trim_start().starts_with('#')
                || line.starts_with(char::is_whitespace)
        })
        .find(|index| lines[*index].starts_with("  tenant:"))
    else {
        return Ok(false);
    };
    let tenant_indent = lines[tenant_index]
        .chars()
        .take_while(|character| character.is_whitespace())
        .count();
    let enable_index = ((tenant_index + 1)..lines.len())
        .take_while(|index| {
            let line = &lines[*index];
            line.trim().is_empty()
                || line
                    .chars()
                    .take_while(|character| character.is_whitespace())
                    .count()
                    > tenant_indent
        })
        .find(|index| lines[*index].trim_start().starts_with("enable:"))
        .ok_or_else(|| format!("{} 缺少 yudao.tenant.enable 配置", path.display()))?;
    let indent = lines[enable_index]
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect::<String>();
    let comment = lines[enable_index]
        .find('#')
        .map(|index| format!(" {}", lines[enable_index][index..].trim_start()))
        .unwrap_or_default();
    lines[enable_index] = format!("{indent}enable: {enabled}{comment}");
    write_lines_preserving_final_newline(path, &text, lines, "写入租户配置")?;
    Ok(true)
}

fn configure_super_admin(sql_root: &Path, username: &str, password: &str) -> Result<(), String> {
    if username.contains(['\r', '\n']) {
        return Err("超管用户名不能包含换行符".to_string());
    }
    let password_hash =
        bcrypt::hash(password, 10).map_err(|e| format!("生成超管密码哈希失败: {e}"))?;
    let escaped_username = username.replace('\'', "''");
    let mut updated_files = 0;
    rewrite_super_admin_sql_files(
        sql_root,
        &escaped_username,
        &password_hash,
        &mut updated_files,
    )?;
    if updated_files == 0 {
        return Err(format!("未在 {} 找到超管初始化 SQL", sql_root.display()));
    }
    Ok(())
}

fn rewrite_super_admin_sql_files(
    root: &Path,
    username: &str,
    password_hash: &str,
    updated_files: &mut usize,
) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|e| format!("读取 SQL 目录失败 {}: {e}", root.display()))?
    {
        let entry = entry.map_err(|e| format!("读取 SQL 目录项失败: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            rewrite_super_admin_sql_files(&path, username, password_hash, updated_files)?;
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("sql") {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("读取 SQL 文件失败 {}: {e}", path.display()))?;
        let mut changed = false;
        let lines = text
            .lines()
            .map(|line| {
                if let Some(updated) = replace_super_admin_insert(line, username, password_hash) {
                    changed = true;
                    updated
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>();
        if changed {
            write_lines_preserving_final_newline(&path, &text, lines, "写入超管初始化 SQL")?;
            *updated_files += 1;
        }
    }
    Ok(())
}

fn replace_super_admin_insert(line: &str, username: &str, password_hash: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if !lower.contains("insert into") || !lower.contains("system_users") {
        return None;
    }
    let values_index = lower
        .find("values (1,")
        .or_else(|| lower.find("values(1,"))?;
    let (username_start, username_end) = sql_string_range(line, values_index)?;
    let (password_start, password_end) = sql_string_range(line, username_end + 1)?;
    Some(format!(
        "{}{}{}{}{}",
        &line[..username_start],
        username,
        &line[username_end..password_start],
        password_hash,
        &line[password_end..]
    ))
}

fn sql_string_range(line: &str, search_from: usize) -> Option<(usize, usize)> {
    let bytes = line.as_bytes();
    let opening = line[search_from..].find('\'')? + search_from;
    let mut index = opening + 1;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            return Some((opening + 1, index));
        }
        index += 1;
    }
    None
}

fn rewrite_text_files(root: &Path, replacements: &[(&str, &str)]) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | "dist" | ".idea" | ".vscode"
        ) {
            continue;
        }
        if path.is_dir() {
            rewrite_text_files(&path, replacements)?;
        } else if should_rewrite_file(&path) {
            let Ok(mut text) = fs::read_to_string(&path) else {
                continue;
            };
            let before = text.clone();
            for (from, to) in replacements {
                text = text.replace(from, to);
            }
            if text != before {
                fs::write(&path, text).map_err(|e| format!("写入文件失败: {e}"))?;
            }
        }
    }
    Ok(())
}

fn relocate_java_packages(root: &Path, base_package: &str) -> Result<(), String> {
    let mut java_roots = Vec::new();
    collect_java_roots(root, &mut java_roots)?;
    let package_path = base_package.replace('.', std::path::MAIN_SEPARATOR_STR);
    for java_root in java_roots {
        let old = java_root.join("cn").join("iocoder").join("yudao");
        if !old.exists() {
            continue;
        }
        let new = java_root.join(&package_path);
        if new == old {
            continue;
        }
        if new.exists() {
            return Err(format!("目标 Java 包目录已存在: {}", new.display()));
        }
        let staging = java_root.join(".yudao-package-relocation");
        if staging.exists() {
            return Err(format!("Java 包迁移暂存目录已存在: {}", staging.display()));
        }
        fs::rename(&old, &staging).map_err(|e| format!("暂存旧 Java 包目录失败: {e}"))?;
        if let Err(error) = copy_dir_contents(&staging, &new) {
            let _ = fs::remove_dir_all(&new);
            let _ = fs::rename(&staging, &old);
            return Err(format!("迁移 Java 包目录失败: {error}"));
        }
        fs::remove_dir_all(&staging).map_err(|e| format!("清理 Java 包暂存目录失败: {e}"))?;
        remove_empty_package_ancestors(old.parent(), &java_root)?;
    }
    Ok(())
}

fn remove_empty_package_ancestors(mut current: Option<&Path>, stop: &Path) -> Result<(), String> {
    while let Some(path) = current {
        if path == stop || !path.starts_with(stop) {
            break;
        }
        let is_empty = fs::read_dir(path)
            .map_err(|e| format!("检查旧 Java 包目录失败 {}: {e}", path.display()))?
            .next()
            .is_none();
        if !is_empty {
            break;
        }
        fs::remove_dir(path)
            .map_err(|e| format!("清理旧 Java 包目录失败 {}: {e}", path.display()))?;
        current = path.parent();
    }
    Ok(())
}

fn collect_java_roots(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), ".git" | "node_modules" | "target") {
            continue;
        }
        if name == "java" && path.join("cn").join("iocoder").join("yudao").exists() {
            out.push(path);
        } else {
            collect_java_roots(&path, out)?;
        }
    }
    Ok(())
}

fn write_scaffold_manifest(output_dir: &Path, payload: &RunPayload) -> Result<(), String> {
    let answers = &payload.answers;
    let selected = serde_json::to_string_pretty(&json!({
        "projectName": &answers.project_name,
        "displayName": &answers.display_name,
        "backend": &answers.backend,
        "jdkVersion": &answers.jdk_version,
        "groupId": &answers.group_id,
        "artifactId": &answers.artifact_id,
        "version": &answers.version,
        "basePackage": &answers.base_package,
        "modules": &answers.modules,
        "frontends": &answers.frontends,
        "tenantEnabled": answers.tenant_enabled,
        "superAdminUsername": &answers.super_admin_username,
        "superAdminPasswordConfigured": !answers.super_admin_password.is_empty(),
        "monolithPort": answers.monolith_port,
        "gatewayPort": answers.gateway_port,
        "microservicePorts": &answers.microservice_ports,
        "vbenVariant": &answers.vben_variant,
        "mirror": &payload.mirror,
    }))
    .map_err(|e| format!("序列化生成配置失败: {e}"))?;

    let readme = format!(
        "# {}\n\n由 yudao-scaffold-ui 生成。\n\n## 目录\n\n- `backend/`: 后端模板\n- `frontend/`: 选中的前端模板\n\n## 生成配置\n\n```json\n{}\n```\n",
        answers.display_name, selected
    );
    fs::write(output_dir.join("README.scaffold.md"), readme)
        .map_err(|e| format!("写入生成说明失败: {e}"))?;
    fs::write(output_dir.join(".scaffold.json"), selected)
        .map_err(|e| format!("写入生成配置失败: {e}"))?;
    Ok(())
}

fn guard_removable_output_dir(path: &Path) -> Result<(), String> {
    let canonical = fs::canonicalize(path).map_err(|e| format!("解析输出目录失败: {e}"))?;
    if canonical.parent().is_none() || canonical.components().count() < 4 {
        return Err(format!("拒绝删除过高层级目录: {}", canonical.display()));
    }
    let home = home_dir()?;
    if let Ok(home) = fs::canonicalize(home) {
        if canonical == home {
            return Err("拒绝删除用户主目录".to_string());
        }
    }
    Ok(())
}

fn unique_sibling_path(target: &Path, kind: &str) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("路径缺少父目录: {}", target.display()))?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("路径缺少有效目录名: {}", target.display()))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("系统时间异常: {e}"))?
        .as_nanos();
    Ok(parent.join(format!(
        ".{name}.yudao-scaffold-{kind}-{}-{nonce}",
        std::process::id()
    )))
}

fn runtime_paths(workspace: Option<String>) -> Result<RuntimePaths, String> {
    let home_dir = home_dir()?;
    let workspace = workspace
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir.join("yudao-scaffold-workspace"));
    let cache_dir = home_dir.join(".yudao-scaffold-ui").join("templates");
    Ok(RuntimePaths {
        home_dir,
        workspace,
        cache_dir,
    })
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "无法确定用户目录".to_string())
}

fn module_meta() -> serde_json::Value {
    json!([
        module(
            "infra",
            "基础设施",
            "配置、文件、代码生成与系统支撑能力",
            &[],
            false,
            false,
            true,
            &[48082],
            None
        ),
        module(
            "system",
            "系统管理",
            "用户、角色、菜单、权限、部门、岗位、字典等基础后台能力",
            &["infra"],
            false,
            false,
            true,
            &[48081],
            None
        ),
        module(
            "member",
            "会员中心",
            "会员、等级、积分、地址等用户侧基础能力",
            &["system"],
            false,
            false,
            false,
            &[48087],
            None
        ),
        module(
            "bpm",
            "工作流程",
            "Flowable 流程定义、任务、审批与表单能力",
            &["system"],
            false,
            false,
            false,
            &[48083],
            None
        ),
        module(
            "pay",
            "支付中心",
            "支付订单、退款、渠道与回调管理",
            &["system"],
            false,
            false,
            false,
            &[48085],
            None
        ),
        module(
            "mp",
            "微信公众号",
            "公众号账号、消息、菜单、素材与粉丝管理",
            &["system"],
            false,
            false,
            false,
            &[48086],
            None
        ),
        module(
            "mall",
            "商城系统",
            "商品、交易、营销、统计等商城业务模块",
            &["system", "member", "pay"],
            true,
            false,
            false,
            &[48100, 48102, 48101, 48103],
            Some(&["product", "trade", "promotion", "statistics"])
        ),
        module(
            "crm",
            "CRM",
            "客户、商机、合同、回款等客户关系管理",
            &["system"],
            false,
            false,
            false,
            &[48089],
            None
        ),
        module(
            "erp",
            "ERP",
            "采购、销售、库存、财务等企业资源管理",
            &["system"],
            false,
            false,
            false,
            &[48088],
            None
        ),
        module(
            "iot",
            "IoT",
            "产品、设备、物模型、消息与规则引擎",
            &["system"],
            false,
            false,
            false,
            &[48091],
            None
        ),
        module(
            "mes",
            "MES",
            "生产计划、工单、质量、设备与车间管理",
            &["system"],
            false,
            false,
            false,
            &[48092],
            None
        ),
        module(
            "report",
            "报表设计",
            "报表、仪表盘与数据可视化能力",
            &["system"],
            false,
            false,
            false,
            &[48084],
            None
        ),
        module(
            "ai",
            "AI 大模型",
            "AI 对话、知识库、绘画与工作流能力，需要 JDK 17",
            &["system"],
            false,
            true,
            false,
            &[48090],
            None
        )
    ])
}

fn module(
    id: &str,
    title: &str,
    description: &str,
    deps: &[&str],
    composite: bool,
    jdk17_only: bool,
    required: bool,
    ports: &[u16],
    subnames: Option<&[&str]>,
) -> serde_json::Value {
    json!({
        "id": id,
        "title": title,
        "description": description,
        "deps": deps,
        "composite": composite,
        "jdk17Only": jdk17_only,
        "required": required,
        "defaultMicroservicePorts": ports,
        "microserviceSubnames": subnames,
    })
}

fn frontend_meta() -> serde_json::Value {
    json!([
        frontend(
            "admin-vue3",
            "Vue3 管理后台",
            "Element Plus 管理后台，适合新项目默认选择",
            "yudao-ui-admin-vue3",
            "admin",
            "admin",
            true
        ),
        frontend(
            "admin-vben",
            "Vben 管理后台",
            "Vben 5 管理后台，支持多套 UI 变体",
            "yudao-ui-admin-vben",
            "admin",
            "admin",
            true
        ),
        frontend(
            "admin-vue2",
            "Vue2 管理后台",
            "Element UI 管理后台，适合维护 Vue2 技术栈",
            "yudao-ui-admin-vue2",
            "admin",
            "admin",
            true
        ),
        frontend(
            "admin-uniapp",
            "移动管理端",
            "uni-app 管理端，支持 H5/小程序/APP",
            "yudao-ui-admin-uniapp",
            "admin",
            "admin-uniapp",
            false
        ),
        frontend(
            "mall-uniapp",
            "商城移动端",
            "商城 uni-app，多端发行",
            "yudao-mall-uniapp",
            "mall",
            "mall",
            false
        ),
        frontend(
            "go-view",
            "GoView 大屏",
            "低代码数据可视化大屏",
            "yudao-ui-go-view",
            "dashboard",
            "dashboard",
            false
        )
    ])
}

fn frontend(
    id: &str,
    title: &str,
    description: &str,
    local: &str,
    role: &str,
    role_suffix: &str,
    modular: bool,
) -> serde_json::Value {
    json!({
        "id": id,
        "title": title,
        "description": description,
        "local": local,
        "role": role,
        "roleSuffix": role_suffix,
        "modular": modular
    })
}

fn template_meta(paths: &RuntimePaths) -> serde_json::Value {
    json!(templates()
        .iter()
        .map(|template| {
            let local = paths.workspace.join(template.name);
            let cache = paths.cache_dir.join(template.name);
            let jdk17_cache = paths
                .cache_dir
                .join(template_cache_dir_name(template.name, Some("master-jdk17")));
            json!({
                "name": template.name,
                "kind": template.kind,
                "localPath": path_string(&local),
                "localPresent": directory_has_entries(&local),
                "cachePath": path_string(&cache),
                "cachePresent": directory_has_entries(&cache),
                "cacheVariants": if template.kind == "backend" {
                    json!({
                        "8": {
                            "path": path_string(&cache),
                            "present": directory_has_entries(&cache),
                        },
                        "17": {
                            "path": path_string(&jdk17_cache),
                            "present": directory_has_entries(&jdk17_cache),
                        }
                    })
                } else {
                    serde_json::Value::Null
                },
                "gitee": template.gitee,
                "github": template.github,
                "isGitRepo": false
            })
        })
        .collect::<Vec<_>>())
}

fn templates() -> &'static [TemplateSource] {
    &[
        TemplateSource {
            name: "ruoyi-vue-pro",
            kind: "backend",
            github: "https://github.com/YunaiV/ruoyi-vue-pro.git",
            gitee: "https://gitee.com/zhijiantianya/ruoyi-vue-pro.git",
        },
        TemplateSource {
            name: "yudao-cloud",
            kind: "backend",
            github: "https://github.com/YunaiV/yudao-cloud.git",
            gitee: "https://gitee.com/zhijiantianya/yudao-cloud.git",
        },
        TemplateSource {
            name: "yudao-ui-admin-vue3",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-ui-admin-vue3.git",
            gitee: "https://gitee.com/yudaocode/yudao-ui-admin-vue3.git",
        },
        TemplateSource {
            name: "yudao-ui-admin-vben",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-ui-admin-vben.git",
            gitee: "https://gitee.com/yudaocode/yudao-ui-admin-vben.git",
        },
        TemplateSource {
            name: "yudao-ui-admin-vue2",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-ui-admin-vue2.git",
            gitee: "https://gitee.com/yudaocode/yudao-ui-admin-vue2.git",
        },
        TemplateSource {
            name: "yudao-ui-admin-uniapp",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-ui-admin-uniapp.git",
            gitee: "https://gitee.com/yudaocode/yudao-ui-admin-uniapp.git",
        },
        TemplateSource {
            name: "yudao-mall-uniapp",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-mall-uniapp.git",
            gitee: "https://gitee.com/yudaocode/yudao-mall-uniapp.git",
        },
        TemplateSource {
            name: "yudao-ui-go-view",
            kind: "frontend",
            github: "https://github.com/yudaocode/yudao-ui-go-view.git",
            gitee: "https://gitee.com/yudaocode/yudao-ui-go-view.git",
        },
    ]
}

fn template_source(name: &str) -> Option<TemplateSource> {
    templates().iter().copied().find(|t| t.name == name)
}

fn frontend_source(id: &str) -> Option<FrontendSource> {
    [
        FrontendSource {
            id: "admin-vue3",
            template: "yudao-ui-admin-vue3",
            role_suffix: "admin",
        },
        FrontendSource {
            id: "admin-vben",
            template: "yudao-ui-admin-vben",
            role_suffix: "admin",
        },
        FrontendSource {
            id: "admin-vue2",
            template: "yudao-ui-admin-vue2",
            role_suffix: "admin",
        },
        FrontendSource {
            id: "admin-uniapp",
            template: "yudao-ui-admin-uniapp",
            role_suffix: "admin-uniapp",
        },
        FrontendSource {
            id: "mall-uniapp",
            template: "yudao-mall-uniapp",
            role_suffix: "mall",
        },
        FrontendSource {
            id: "go-view",
            template: "yudao-ui-go-view",
            role_suffix: "dashboard",
        },
    ]
    .into_iter()
    .find(|f| f.id == id)
}

fn archive_url_candidates(url: &str, branch: Option<&str>) -> Vec<String> {
    let trimmed = url.trim().trim_end_matches(".git").trim_end_matches('/');
    if trimmed.ends_with(".zip") {
        return vec![trimmed.to_string()];
    }
    let branches = branch
        .map(|branch| vec![branch])
        .unwrap_or_else(|| vec!["master", "main"]);
    if trimmed.contains("github.com/") {
        return branches
            .iter()
            .map(|branch| format!("{trimmed}/archive/refs/heads/{branch}.zip"))
            .collect();
    }
    if trimmed.contains("gitee.com/") {
        return branches
            .iter()
            .map(|branch| format!("{trimmed}/repository/archive/{branch}.zip"))
            .collect();
    }
    Vec::new()
}

fn should_rewrite_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()).unwrap_or(""),
        "java"
            | "kt"
            | "xml"
            | "yml"
            | "yaml"
            | "properties"
            | "md"
            | "json"
            | "ts"
            | "js"
            | "vue"
            | "html"
            | "sql"
            | "env"
            | "txt"
    )
}

fn strip_first_component(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components().skip(1) {
        if let Component::Normal(s) = component {
            out.push(s);
        }
    }
    out
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn emit_phase(app: &AppHandle, index: usize, total: usize, title: &str) {
    let _ = app.emit(
        "scaffold-event",
        json!({ "type": "phase", "index": index, "total": total, "title": title }),
    );
}

fn emit_info(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "scaffold-event",
        json!({ "type": "info", "message": message }),
    );
}

fn emit_ok(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "scaffold-event",
        json!({ "type": "ok", "message": message }),
    );
}

fn emit_warn(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "scaffold-event",
        json!({ "type": "warn", "message": message }),
    );
}

fn emit_done(app: &AppHandle, output_dir: &str) {
    let _ = app.emit(
        "scaffold-event",
        json!({ "type": "done", "outputDir": output_dir }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use zip::write::SimpleFileOptions;

    #[test]
    fn recognizes_supported_zip_signatures() {
        assert!(has_zip_signature(b"PK\x03\x04archive"));
        assert!(has_zip_signature(b"PK\x05\x06empty"));
        assert!(has_zip_signature(b"PK\x07\x08spanned"));
        assert!(!has_zip_signature(b"<!doctype html>"));
        assert!(!has_zip_signature(b"PK"));
    }

    #[test]
    fn builds_master_and_main_archive_candidates() {
        assert_eq!(
            archive_url_candidates("https://github.com/example/repo.git", None),
            vec![
                "https://github.com/example/repo/archive/refs/heads/master.zip",
                "https://github.com/example/repo/archive/refs/heads/main.zip",
            ]
        );
        assert_eq!(
            archive_url_candidates("https://gitee.com/example/repo.git", None),
            vec![
                "https://gitee.com/example/repo/repository/archive/master.zip",
                "https://gitee.com/example/repo/repository/archive/main.zip",
            ]
        );
        assert_eq!(
            archive_url_candidates("https://github.com/example/repo.git", Some("master-jdk17")),
            vec!["https://github.com/example/repo/archive/refs/heads/master-jdk17.zip"]
        );
        assert_eq!(backend_template_branch("8").unwrap(), "master");
        assert_eq!(backend_template_branch("17").unwrap(), "master-jdk17");
        assert_eq!(
            template_cache_dir_name("ruoyi-vue-pro", Some("master-jdk17")),
            "ruoyi-vue-pro-jdk17"
        );
    }

    #[test]
    fn keeps_only_system_and_infra_when_they_are_the_only_selection() {
        let test_root = unique_test_dir();
        fs::create_dir_all(test_root.join("yudao-server")).unwrap();
        for module in ["system", "infra", "member", "ai", "wms"] {
            fs::create_dir_all(test_root.join(format!("yudao-module-{module}"))).unwrap();
        }
        fs::write(
            test_root.join("pom.xml"),
            r#"<modules>
  <module>yudao-module-system</module>
  <module>yudao-module-infra</module>
<!--  <module>yudao-module-member</module>-->
<!--  <module>yudao-module-ai</module>-->
</modules>
"#,
        )
        .unwrap();
        fs::write(
            test_root.join("yudao-server").join("pom.xml"),
            r#"<dependencies>
<!--  <dependency>-->
<!--    <artifactId>yudao-module-member</artifactId>-->
<!--  </dependency>-->
<!--  <dependency>-->
<!--    <artifactId>yudao-module-ai</artifactId>-->
<!--  </dependency>-->
</dependencies>
"#,
        )
        .unwrap();

        let selected = vec!["system".into(), "infra".into()];
        prune_backend_modules(&test_root, &selected).unwrap();

        assert!(test_root.join("yudao-module-system").is_dir());
        assert!(test_root.join("yudao-module-infra").is_dir());
        assert!(!test_root.join("yudao-module-member").exists());
        assert!(!test_root.join("yudao-module-ai").exists());
        assert!(!test_root.join("yudao-module-wms").exists());

        let root_pom = fs::read_to_string(test_root.join("pom.xml")).unwrap();
        assert!(root_pom.contains("<!--  <module>yudao-module-member</module>-->"));
        assert!(root_pom.contains("<!--  <module>yudao-module-ai</module>-->"));
        let server_pom =
            fs::read_to_string(test_root.join("yudao-server").join("pom.xml")).unwrap();
        assert!(server_pom.contains("<!--    <artifactId>yudao-module-member</artifactId>-->"));
        assert!(server_pom.contains("<!--    <artifactId>yudao-module-ai</artifactId>-->"));

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn activates_a_selected_optional_module_in_both_poms() {
        let test_root = unique_test_dir();
        fs::create_dir_all(test_root.join("yudao-server")).unwrap();
        for module in ["system", "infra", "member"] {
            fs::create_dir_all(test_root.join(format!("yudao-module-{module}"))).unwrap();
        }
        fs::write(
            test_root.join("pom.xml"),
            "<module>yudao-module-system</module>\n<module>yudao-module-infra</module>\n<!-- <module>yudao-module-member</module> -->\n",
        )
        .unwrap();
        fs::write(
            test_root.join("yudao-server").join("pom.xml"),
            "<!-- <dependency> -->\n<!-- <artifactId>yudao-module-member</artifactId> -->\n<!-- </dependency> -->\n",
        )
        .unwrap();

        prune_backend_modules(
            &test_root,
            &["system".into(), "infra".into(), "member".into()],
        )
        .unwrap();

        let root_pom = fs::read_to_string(test_root.join("pom.xml")).unwrap();
        assert!(root_pom.contains("<module>yudao-module-member</module>"));
        assert!(!root_pom.contains("<!-- <module>yudao-module-member"));
        let server_pom =
            fs::read_to_string(test_root.join("yudao-server").join("pom.xml")).unwrap();
        assert!(server_pom.contains("<artifactId>yudao-module-member</artifactId>"));
        assert!(!server_pom.contains("<!-- <artifactId>yudao-module-member"));

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn applies_maven_port_tenant_and_super_admin_settings() {
        let test_root = unique_test_dir();
        let resources = test_root.join("resources");
        let sql_dir = test_root.join("sql");
        fs::create_dir_all(&resources).unwrap();
        fs::create_dir_all(&sql_dir).unwrap();
        let profile = resources.join("application-local.yaml");
        fs::write(
            &profile,
            "server:\n  port: 48080\nsecurity:\n  frame: localhost:48080 127.0.0.1:48080\n",
        )
        .unwrap();
        let application = resources.join("application.yaml");
        fs::write(
            &application,
            "yudao:\n  tenant: # tenant settings\n    enable: true # current\n",
        )
        .unwrap();
        let pom = test_root.join("pom.xml");
        fs::write(&pom, "<properties><revision>old</revision></properties>\n").unwrap();
        let sql = sql_dir.join("schema.sql");
        fs::write(
            &sql,
            "INSERT INTO system_users (id, username, password) VALUES (1, N'admin', N'old-hash');\n",
        )
        .unwrap();

        set_top_level_server_port(&profile, 49090).unwrap();
        set_tenant_enabled(&application, false).unwrap();
        set_xml_tag_value(&pom, "revision", "2.3.4-SNAPSHOT").unwrap();
        configure_super_admin(&sql_dir, "root'user", "new-secret").unwrap();

        let profile_text = fs::read_to_string(profile).unwrap();
        assert!(profile_text.contains("port: 49090"));
        assert!(profile_text.contains("localhost:49090 127.0.0.1:49090"));
        assert!(fs::read_to_string(application)
            .unwrap()
            .contains("enable: false # current"));
        assert!(fs::read_to_string(pom)
            .unwrap()
            .contains("<revision>2.3.4-SNAPSHOT</revision>"));
        let sql_text = fs::read_to_string(sql).unwrap();
        assert!(sql_text.contains("N'root''user'"));
        let (_, username_end) =
            sql_string_range(&sql_text, sql_text.find("VALUES").unwrap()).unwrap();
        let (password_start, password_end) = sql_string_range(&sql_text, username_end + 1).unwrap();
        assert!(bcrypt::verify("new-secret", &sql_text[password_start..password_end]).unwrap());

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn relocates_java_packages_without_deleting_cn_targets_or_recursing() {
        for base_package in ["cn.example.app", "cn.iocoder.yudao.child"] {
            let test_root = unique_test_dir();
            let java_root = test_root
                .join("module")
                .join("src")
                .join("main")
                .join("java");
            let old_package = java_root.join("cn").join("iocoder").join("yudao");
            fs::create_dir_all(&old_package).unwrap();
            fs::write(old_package.join("Example.java"), "class Example {}\n").unwrap();

            relocate_java_packages(&test_root, base_package).unwrap();

            let new_package =
                java_root.join(base_package.replace('.', std::path::MAIN_SEPARATOR_STR));
            assert_eq!(
                fs::read_to_string(new_package.join("Example.java")).unwrap(),
                "class Example {}\n"
            );
            assert!(!java_root.join(".yudao-package-relocation").exists());

            fs::remove_dir_all(test_root).unwrap();
        }
    }

    #[test]
    fn staged_output_preserves_existing_content_until_commit() {
        let test_root = unique_test_dir();
        let target = test_root.join("project");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old.txt"), "old").unwrap();

        {
            let staged = StagedOutputDir::new(&target).unwrap();
            fs::write(staged.path().join("abandoned.txt"), "abandoned").unwrap();
        }
        assert_eq!(fs::read_to_string(target.join("old.txt")).unwrap(), "old");

        let mut staged = StagedOutputDir::new(&target).unwrap();
        fs::write(staged.path().join("new.txt"), "new").unwrap();
        assert!(staged.commit(&target).unwrap().is_none());
        assert!(!target.join("old.txt").exists());
        assert_eq!(fs::read_to_string(target.join("new.txt")).unwrap(), "new");

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn keeps_only_the_selected_vben_variant() {
        let test_root = unique_test_dir();
        for variant in ["antd", "antdv-next", "ele", "naive", "tdesign"] {
            fs::create_dir_all(test_root.join(format!("web-{variant}"))).unwrap();
        }

        prune_vben_variants(&test_root, "ele").unwrap();

        assert!(test_root.join("web-ele").is_dir());
        assert!(!test_root.join("web-antd").exists());
        assert!(!test_root.join("web-antdv-next").exists());
        assert!(!test_root.join("web-naive").exists());
        assert!(!test_root.join("web-tdesign").exists());

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn applies_microservice_port_to_yaml_and_dockerfile() {
        let test_root = unique_test_dir();
        let server = test_root.join("module-server");
        let resources = server.join("src").join("main").join("resources");
        fs::create_dir_all(&resources).unwrap();
        fs::write(
            resources.join("application.yaml"),
            "spring:\n  application:\n    name: demo\nserver:\n  port: 48081\n",
        )
        .unwrap();
        fs::write(server.join("Dockerfile"), "FROM scratch\nEXPOSE 48081\n").unwrap();

        configure_microservice_port(&resources, 49101).unwrap();

        assert!(fs::read_to_string(resources.join("application.yaml"))
            .unwrap()
            .contains("port: 49101"));
        assert!(fs::read_to_string(server.join("Dockerfile"))
            .unwrap()
            .contains("EXPOSE 49101"));
        assert_eq!(default_microservice_ports("member"), &[48087]);
        assert_eq!(
            default_microservice_ports("mall"),
            &[48100, 48102, 48101, 48103]
        );

        fs::remove_dir_all(test_root).unwrap();
    }

    #[tokio::test]
    async fn falls_back_when_a_successful_response_is_not_a_zip() {
        let zip_bytes = test_zip();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response_body in [b"<!doctype html>blocked".to_vec(), zip_bytes] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 1024];
                let _ = socket.read(&mut request).await.unwrap();
                let content_type = if has_zip_signature(&response_body) {
                    "application/zip"
                } else {
                    "text/html"
                };
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response_body.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(&response_body).await.unwrap();
            }
        });

        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        let destination = test_root.join("template-cache");
        let candidates = vec![
            format!("http://{address}/not-a-zip"),
            format!("http://{address}/template.zip"),
        ];
        let client = reqwest::Client::new();

        download_and_extract(&client, &candidates, &destination)
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("README.md")).unwrap(),
            "template contents"
        );
        assert!(!test_root.join(".template-cache.zip").exists());
        assert!(!test_root.join(".template-cache.extracting").exists());

        fs::remove_dir_all(test_root).unwrap();
    }

    fn test_zip() -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("repository-root/README.md", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"template contents").unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn unique_test_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "yudao-scaffold-download-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
