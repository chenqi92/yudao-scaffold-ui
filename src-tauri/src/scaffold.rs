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
    #[serde(default)]
    git_remotes: Vec<GitRemote>,
    modules: Vec<String>,
    frontends: Vec<String>,
    monolith_port: Option<u16>,
    gateway_port: Option<u16>,
    #[serde(default)]
    microservice_ports: HashMap<String, Vec<u16>>,
    super_admin_username: String,
    super_admin_password: String,
    #[serde(default)]
    database: DatabaseSettings,
    #[serde(default)]
    redis: RedisSettings,
    pull_existing: bool,
    force: Option<bool>,
    tenant_enabled: bool,
    vben_variant: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitRemote {
    name: String,
    url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseSettings {
    enabled: bool,
    url: String,
    username: String,
    password: String,
    slave_enabled: bool,
    slave_url: String,
    slave_username: String,
    slave_password: String,
}

#[derive(Debug, Default, Deserialize)]
struct RedisSettings {
    enabled: bool,
    host: String,
    port: u16,
    database: u8,
    password: String,
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
    if output_dir.exists() && !output_dir.is_dir() {
        return Err("输出路径已存在，但不是目录".to_string());
    }
    fs::create_dir_all(&output_dir).map_err(|e| format!("创建所选输出目录失败: {e}"))?;

    let backend_target = output_dir.join("backend");
    let frontend_target = output_dir.join("frontend");
    let managed_output_exists = backend_target.exists() || frontend_target.exists();
    if managed_output_exists && answers.force != Some(true) {
        return Err("所选目录中已存在 backend/ 或 frontend/，请确认覆盖后再生成".to_string());
    }
    for target in [&backend_target, &frontend_target] {
        if target.exists() {
            if !target.is_dir() {
                return Err(format!("受管理输出路径不是目录: {}", target.display()));
            }
            guard_removable_output_dir(target)?;
        }
    }
    let mut staged_backend = StagedOutputDir::new(&backend_target)?;
    let mut staged_frontend = if answers.frontends.is_empty() {
        None
    } else {
        Some(StagedOutputDir::new(&frontend_target)?)
    };

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
    let backend_dst = staged_backend.path().to_path_buf();
    copy_dir_contents(&backend_src, &backend_dst).map_err(|e| format!("复制后端模板失败: {e}"))?;
    clean_backend_template(&backend_dst)?;
    prune_backend_modules(&backend_dst, &answers.modules)?;
    let removed_sql_rows = customize_backend_tree(&backend_dst, answers)?;
    if removed_sql_rows > 0 {
        emit_info(
            &app,
            &format!("已从初始化 SQL 裁剪 {removed_sql_rows} 条未选模块数据"),
        );
    }
    let optional_modules = answers
        .modules
        .iter()
        .filter(|module| !matches!(module.as_str(), "system" | "infra"))
        .cloned()
        .collect::<Vec<_>>();
    if !optional_modules.is_empty() {
        emit_warn(
            &app,
            &format!(
                "所选可选模块 {} 的完整生产 SQL 不随官方开源模板提供，请按对应模块官方文档另行导入",
                optional_modules.join(", ")
            ),
        );
    }
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
        let dst = staged_frontend
            .as_ref()
            .expect("frontend staging exists when a frontend is selected")
            .path()
            .join(frontend.role_suffix);
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
    write_project_readme(&backend_dst, answers)?;
    initialize_git_repository(&backend_dst, &answers.git_remotes)?;

    if let Some(warning) = staged_backend.commit(&backend_target)? {
        emit_warn(&app, &warning);
    }
    if let Some(frontend) = staged_frontend.as_mut() {
        if let Some(warning) = frontend.commit(&frontend_target)? {
            emit_warn(&app, &warning);
        }
    } else if frontend_target.exists() {
        fs::remove_dir_all(&frontend_target)
            .map_err(|e| format!("删除未选择的 frontend/ 失败: {e}"))?;
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
            ".git"
                | ".gitee"
                | ".github"
                | ".image"
                | "node_modules"
                | "target"
                | ".idea"
                | ".vscode"
                | "yudao-ui"
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

fn clean_backend_template(root: &Path) -> Result<(), String> {
    for relative in ["yudao-ui", ".gitee", ".github", ".image"] {
        let path = root.join(relative);
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("清理模板目录 {} 失败: {e}", path.display()))?;
        }
    }

    let infra_java = root
        .join("yudao-module-infra")
        .join("src")
        .join("main")
        .join("java")
        .join("cn")
        .join("iocoder")
        .join("yudao")
        .join("module")
        .join("infra");
    for relative in [
        "controller/admin/demo",
        "dal/dataobject/demo",
        "dal/mysql/demo",
        "service/demo",
    ] {
        let path = infra_java.join(relative);
        if path.exists() {
            fs::remove_dir_all(&path)
                .map_err(|e| format!("清理演示代码 {} 失败: {e}", path.display()))?;
        }
    }
    Ok(())
}

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

fn customize_backend_tree(root: &Path, answers: &ScaffoldAnswers) -> Result<usize, String> {
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
    set_maven_revision_properties(root, &answers.version)?;
    relocate_java_packages(root, &answers.base_package)?;
    configure_backend_settings(root, answers)?;
    prune_unselected_module_configs(root, &answers.modules)?;
    let removed_sql_rows = filter_unselected_module_sql(&root.join("sql"), &answers.modules)?;
    rename_backend_project_identifiers(root, &answers.artifact_id)?;
    extend_generated_gitignore(root)?;
    Ok(removed_sql_rows)
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

fn set_maven_revision_properties(root: &Path, version: &str) -> Result<(), String> {
    for relative in ["pom.xml", "yudao-dependencies/pom.xml"] {
        let path = root.join(relative);
        if path.is_file() {
            set_xml_tag_value(&path, "revision", version)?;
        }
    }
    Ok(())
}

fn configure_backend_settings(root: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    if answers.backend == "monolith" {
        let port = answers
            .monolith_port
            .ok_or_else(|| "单体项目缺少服务端口".to_string())?;
        for profile in ["application-local.yaml", "application-dev.yaml"] {
            let path = root
                .join("yudao-server")
                .join("src")
                .join("main")
                .join("resources")
                .join(profile);
            set_top_level_server_port(&path, port)?;
            configure_connection_profile(&path, &answers.database, &answers.redis)?;
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
            for profile in ["application-local.yaml", "application-dev.yaml"] {
                configure_connection_profile(
                    &resources.join(profile),
                    &answers.database,
                    &answers.redis,
                )?;
            }
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

fn configure_connection_profile(
    path: &Path,
    database: &DatabaseSettings,
    redis: &RedisSettings,
) -> Result<(), String> {
    if database.enabled {
        configure_database_yaml(path, database)?;
    } else if !database.slave_enabled {
        remove_disabled_slave_yaml(path)?;
    }
    if redis.enabled {
        configure_redis_yaml(path, redis)?;
    }
    Ok(())
}

fn remove_disabled_slave_yaml(path: &Path) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取数据库配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    remove_yaml_mapping_block(&mut lines, "slave", 8);
    write_lines_preserving_final_newline(path, &text, lines, "移除未启用的从库配置")
}

fn configure_database_yaml(path: &Path, settings: &DatabaseSettings) -> Result<(), String> {
    if settings.url.trim().is_empty() || settings.username.trim().is_empty() {
        return Err("启用自定义数据库后，主库 URL 和账号不能为空".to_string());
    }
    if settings.slave_enabled
        && (settings.slave_url.trim().is_empty() || settings.slave_username.trim().is_empty())
    {
        return Err("启用从库后，从库 URL 和账号不能为空".to_string());
    }
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取数据库配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    update_datasource_block(
        &mut lines,
        "master",
        &settings.url,
        &settings.username,
        &settings.password,
    )?;
    if settings.slave_enabled {
        update_datasource_block(
            &mut lines,
            "slave",
            &settings.slave_url,
            &settings.slave_username,
            &settings.slave_password,
        )?;
    } else {
        remove_yaml_mapping_block(&mut lines, "slave", 8);
    }
    write_lines_preserving_final_newline(path, &text, lines, "写入数据库配置")
}

fn update_datasource_block(
    lines: &mut [String],
    name: &str,
    url: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    let marker = format!("        {name}:");
    let start = lines
        .iter()
        .position(|line| line.starts_with(&marker))
        .ok_or_else(|| format!("模板缺少 {name} 数据源配置"))?;
    for (key, value) in [("url", url), ("username", username), ("password", password)] {
        let property = format!("          {key}:");
        let index = ((start + 1)..lines.len())
            .take_while(|index| {
                let line = &lines[*index];
                line.trim().is_empty()
                    || line
                        .chars()
                        .take_while(|character| character.is_whitespace())
                        .count()
                        > 8
            })
            .find(|index| lines[*index].starts_with(&property))
            .ok_or_else(|| format!("模板的 {name} 数据源缺少 {key} 配置"))?;
        lines[index] = format!("{property} {}", yaml_string(value));
    }
    Ok(())
}

fn remove_yaml_mapping_block(lines: &mut Vec<String>, name: &str, indent: usize) {
    let marker = format!("{}{name}:", " ".repeat(indent));
    let Some(start) = lines.iter().position(|line| line.starts_with(&marker)) else {
        return;
    };
    let mut end = start + 1;
    while end < lines.len() {
        let line = &lines[end];
        if line.trim().is_empty()
            || line.trim_start().starts_with('#')
            || line
                .chars()
                .take_while(|character| character.is_whitespace())
                .count()
                > indent
        {
            end += 1;
        } else {
            break;
        }
    }
    lines.drain(start..end);
}

fn remove_empty_yaml_mapping_blocks(lines: &mut Vec<String>, name: &str, indent: usize) {
    let marker = format!("{}{name}:", " ".repeat(indent));
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim_end() != marker {
            index += 1;
            continue;
        }
        let has_child = lines[(index + 1)..]
            .iter()
            .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .is_some_and(|line| {
                line.chars()
                    .take_while(|character| character.is_whitespace())
                    .count()
                    > indent
            });
        if has_child {
            index += 1;
        } else {
            lines.remove(index);
        }
    }
}

fn prune_unselected_module_configs(root: &Path, selected_modules: &[String]) -> Result<(), String> {
    let selected = selected_modules
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    prune_module_configs_in_tree(root, &selected)
}

fn prune_module_configs_in_tree(root: &Path, selected: &HashSet<&str>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in
        fs::read_dir(root).map_err(|e| format!("读取配置目录失败 {}: {e}", root.display()))?
    {
        let entry = entry.map_err(|e| format!("读取配置目录项失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(name.as_ref(), ".git" | "node_modules" | "target") {
                prune_module_configs_in_tree(&path, selected)?;
            }
            continue;
        }
        let is_application_yaml = name.starts_with("application")
            && matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("yaml" | "yml")
            );
        if is_application_yaml {
            prune_module_config_file(&path, selected)?;
        }
    }
    Ok(())
}

fn prune_module_config_file(path: &Path, selected: &HashSet<&str>) -> Result<(), String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取模块配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();

    // 演示代码已从生成项目中删除，对应配置也不应残留。
    remove_yaml_mapping_block(&mut lines, "demo", 2);
    if !selected.contains("bpm") {
        remove_yaml_mapping_block(&mut lines, "flowable", 0);
    }
    if !selected.contains("mp") && !selected.contains("mall") {
        minimize_required_wx_config(&mut lines);
    }
    if !selected.contains("pay") {
        remove_yaml_mapping_block(&mut lines, "pay", 2);
    }
    if !selected.contains("ai") {
        // 同时覆盖 spring.ai 与 yudao.ai；两者在模板中均为二级配置块。
        while contains_yaml_mapping_block(&lines, "ai", 2) {
            remove_yaml_mapping_block(&mut lines, "ai", 2);
        }
    }
    if !selected.contains("mall") {
        for name in [
            "trade",
            "wxa-code",
            "wxa-subscribe-message",
            "tencent-lbs-key",
        ] {
            remove_yaml_mapping_block(&mut lines, name, 2);
        }
    }
    if !selected.contains("iot") {
        remove_yaml_mapping_block(&mut lines, "iot", 2);
    }
    for parent in ["spring", "yudao"] {
        remove_empty_yaml_mapping_blocks(&mut lines, parent, 0);
    }

    write_lines_preserving_final_newline(path, &text, lines, "裁剪未选模块配置")
}

fn contains_yaml_mapping_block(lines: &[String], name: &str, indent: usize) -> bool {
    let marker = format!("{}{name}:", " ".repeat(indent));
    lines.iter().any(|line| line.starts_with(&marker))
}

fn minimize_required_wx_config(lines: &mut Vec<String>) {
    let Some(start) = lines.iter().position(|line| line.starts_with("wx:")) else {
        return;
    };
    let mut end = start + 1;
    while end < lines.len() {
        let line = &lines[end];
        if line.trim().is_empty()
            || line.trim_start().starts_with('#')
            || line.starts_with(char::is_whitespace)
        {
            end += 1;
        } else {
            break;
        }
    }
    let minimal = [
        "wx:",
        "  mp:",
        "    app-id: ${WX_MP_APP_ID:disabled}",
        "    secret: ${WX_MP_SECRET:disabled}",
        "    config-storage:",
        "      type: RedisTemplate",
        "      key-prefix: wx",
        "      http-client-type: HttpComponents",
        "  miniapp:",
        "    appid: ${WX_MINIAPP_APP_ID:disabled}",
        "    secret: ${WX_MINIAPP_SECRET:disabled}",
        "    config-storage:",
        "      type: RedisTemplate",
        "      key-prefix: wa",
        "      http-client-type: HttpComponents",
        "",
    ]
    .into_iter()
    .map(str::to_string);
    lines.splice(start..end, minimal);
}

fn configure_redis_yaml(path: &Path, settings: &RedisSettings) -> Result<(), String> {
    if settings.host.trim().is_empty() {
        return Err("启用自定义 Redis 后，Redis 地址不能为空".to_string());
    }
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取 Redis 配置失败 {}: {e}", path.display()))?;
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| line == "    redis:")
        .ok_or_else(|| format!("{} 缺少 spring.data.redis 配置", path.display()))?;
    for (key, value) in [
        ("host", yaml_string(&settings.host)),
        ("port", settings.port.to_string()),
        ("database", settings.database.to_string()),
    ] {
        let property = format!("      {key}:");
        let index = ((start + 1)..lines.len())
            .take_while(|index| {
                let line = &lines[*index];
                line.trim().is_empty()
                    || line.starts_with(char::is_whitespace)
                    || line.trim_start().starts_with('#')
            })
            .find(|index| lines[*index].starts_with(&property))
            .ok_or_else(|| format!("模板的 Redis 配置缺少 {key}"))?;
        lines[index] = format!("{property} {value}");
    }
    let password_index = ((start + 1)..lines.len())
        .take_while(|index| {
            let line = &lines[*index];
            line.trim().is_empty()
                || line.starts_with(char::is_whitespace)
                || line.trim_start().starts_with('#')
        })
        .find(|index| lines[*index].contains("password:"));
    if settings.password.is_empty() {
        if let Some(index) = password_index {
            if !lines[index].trim_start().starts_with('#') {
                lines[index] = "#      password:".to_string();
            }
        }
    } else if let Some(index) = password_index {
        lines[index] = format!("      password: {}", yaml_string(&settings.password));
    } else {
        lines.insert(
            start + 4,
            format!("      password: {}", yaml_string(&settings.password)),
        );
    }
    write_lines_preserving_final_newline(path, &text, lines, "写入 Redis 配置")
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
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
    let tenant_index = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim() == "yudao:" && !line.starts_with(char::is_whitespace))
        .find_map(|(yudao_index, _)| {
            ((yudao_index + 1)..lines.len())
                .take_while(|index| {
                    let line = &lines[*index];
                    line.trim().is_empty()
                        || line.trim_start().starts_with('#')
                        || line.starts_with(char::is_whitespace)
                })
                .find(|index| lines[*index].starts_with("  tenant:"))
        });
    let Some(tenant_index) = tenant_index else {
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

#[derive(Debug)]
struct SqlInsert {
    table: String,
    columns: Vec<String>,
    values: Vec<String>,
}

impl SqlInsert {
    fn value(&self, column: &str) -> Option<&str> {
        let index = self
            .columns
            .iter()
            .position(|candidate| candidate == column)?;
        self.values.get(index).map(String::as_str)
    }

    fn string_value(&self, column: &str) -> Option<String> {
        unquote_sql_string(self.value(column)?)
    }

    fn integer_value(&self, column: &str) -> Option<u64> {
        self.value(column)?.trim().parse().ok()
    }
}

fn filter_unselected_module_sql(
    sql_root: &Path,
    selected_modules: &[String],
) -> Result<usize, String> {
    let selected = selected_modules
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut removed_rows = 0;
    filter_sql_tree(sql_root, &selected, &mut removed_rows)?;
    Ok(removed_rows)
}

fn filter_sql_tree(
    root: &Path,
    selected: &HashSet<&str>,
    removed_rows: &mut usize,
) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|e| format!("读取 SQL 目录失败 {}: {e}", root.display()))?
    {
        let entry = entry.map_err(|e| format!("读取 SQL 目录项失败: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            filter_sql_tree(&path, selected, removed_rows)?;
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("sql")
            || path.file_name().and_then(|value| value.to_str()) == Some("quartz.sql")
        {
            continue;
        }
        *removed_rows += filter_sql_file(&path, selected)?;
    }
    Ok(())
}

fn filter_sql_file(path: &Path, selected: &HashSet<&str>) -> Result<usize, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("读取 SQL 文件失败 {}: {e}", path.display()))?;
    let (lines, removed_demo_sections) = strip_demo_sql_sections(&text);
    let records = lines
        .iter()
        .map(|line| parse_sql_insert(line))
        .collect::<Vec<_>>();

    let mut menu_parents = HashMap::new();
    let mut removed_menu_ids = HashSet::new();
    let mut removed_codegen_table_ids = HashSet::new();
    for record in records.iter().flatten() {
        if record.table == "infra_codegen_table"
            && record
                .string_value("table_name")
                .is_some_and(|name| name.to_ascii_lowercase().starts_with("yudao_demo"))
        {
            if let Some(id) = record.integer_value("id") {
                removed_codegen_table_ids.insert(id);
            }
        }
        if record.table != "system_menu" {
            continue;
        }
        let (Some(id), Some(parent_id)) = (
            record.integer_value("id"),
            record.integer_value("parent_id"),
        ) else {
            continue;
        };
        menu_parents.insert(id, parent_id);
        if record_is_demo_menu(record)
            || record_module(record).is_some_and(|module| !selected.contains(module))
        {
            removed_menu_ids.insert(id);
        }
    }
    loop {
        let before = removed_menu_ids.len();
        for (id, parent_id) in &menu_parents {
            if removed_menu_ids.contains(parent_id) {
                removed_menu_ids.insert(*id);
            }
        }
        if removed_menu_ids.len() == before {
            break;
        }
    }

    let mut removed = 0;
    let filtered = lines
        .into_iter()
        .zip(records)
        .filter_map(|(line, record)| {
            let should_remove = record
                .as_ref()
                .is_some_and(|record| match record.table.as_str() {
                    "system_menu" => record
                        .integer_value("id")
                        .is_some_and(|id| removed_menu_ids.contains(&id)),
                    "system_role_menu" => record
                        .integer_value("menu_id")
                        .is_some_and(|id| removed_menu_ids.contains(&id)),
                    "system_dict_type" | "system_dict_data" => record
                        .string_value("dict_type")
                        .or_else(|| record.string_value("type"))
                        .and_then(|dict_type| module_for_dict_type(&dict_type))
                        .is_some_and(|module| !selected.contains(module)),
                    "infra_job" => record
                        .string_value("handler_name")
                        .and_then(|handler| module_for_job_handler(&handler))
                        .is_some_and(|module| !selected.contains(module)),
                    "infra_codegen_table" => record
                        .integer_value("id")
                        .is_some_and(|id| removed_codegen_table_ids.contains(&id)),
                    "infra_codegen_column" => record
                        .integer_value("table_id")
                        .is_some_and(|id| removed_codegen_table_ids.contains(&id)),
                    _ => false,
                });
            if should_remove {
                removed += 1;
                None
            } else {
                Some(line)
            }
        })
        .collect::<Vec<_>>();

    if removed > 0 || removed_demo_sections > 0 {
        write_lines_preserving_final_newline(path, &text, filtered, "写入裁剪后的 SQL")?;
    }
    Ok(removed + removed_demo_sections)
}

fn strip_demo_sql_sections(text: &str) -> (Vec<String>, usize) {
    let mut kept = Vec::new();
    let mut skipping = false;
    let mut passed_header_separator = false;
    let mut removed_sections = 0;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let marker = line.trim() == "-- ----------------------------";
        if !skipping
            && ((lower.contains("table structure for") || lower.contains("records of"))
                && lower.contains("yudao_demo"))
        {
            if kept.last().is_some_and(|previous: &String| {
                previous.trim() == "-- ----------------------------"
            }) {
                kept.pop();
            }
            skipping = true;
            passed_header_separator = false;
            removed_sections += 1;
            continue;
        }
        if skipping {
            if marker {
                if passed_header_separator {
                    skipping = false;
                    kept.push(line.to_string());
                } else {
                    passed_header_separator = true;
                }
            }
            continue;
        }
        kept.push(line.to_string());
    }
    (kept, removed_sections)
}

fn parse_sql_insert(line: &str) -> Option<SqlInsert> {
    let lower = line.to_ascii_lowercase();
    let insert_start = lower.find("insert into")? + "insert into".len();
    let columns_start = line[insert_start..].find('(')? + insert_start;
    let table_token = line[insert_start..columns_start].trim();
    let table = table_token
        .rsplit('.')
        .next()?
        .trim_matches(|character| matches!(character, '`' | '"' | '[' | ']'))
        .to_ascii_lowercase();
    let columns_end = line[columns_start + 1..].find(')')? + columns_start + 1;
    let columns = line[columns_start + 1..columns_end]
        .split(',')
        .map(|column| {
            column
                .trim()
                .trim_matches(|character| matches!(character, '`' | '"' | '[' | ']'))
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    let values_keyword = lower[columns_end + 1..].find("values")? + columns_end + 1;
    let values_start = line[values_keyword..].find('(')? + values_keyword;
    let values_end = matching_closing_parenthesis(line, values_start)?;
    let values = parse_sql_csv(&line[values_start + 1..values_end]);
    if columns.len() != values.len() {
        return None;
    }
    Some(SqlInsert {
        table,
        columns,
        values,
    })
}

fn matching_closing_parenthesis(text: &str, opening: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut quoted = false;
    let mut index = opening;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => index += 2,
            b'\'' if quoted && bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => {
                quoted = !quoted;
                index += 1;
            }
            b'(' if !quoted => {
                depth += 1;
                index += 1;
            }
            b')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn parse_sql_csv(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut values = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quoted = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => index += 2,
            b'\'' if quoted && bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => {
                quoted = !quoted;
                index += 1;
            }
            b'(' if !quoted => {
                depth += 1;
                index += 1;
            }
            b')' if !quoted => {
                depth -= 1;
                index += 1;
            }
            b',' if !quoted && depth == 0 => {
                values.push(text[start..index].trim().to_string());
                start = index + 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    values.push(text[start..].trim().to_string());
    values
}

fn unquote_sql_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let quoted = trimmed
        .strip_prefix("N'")
        .or_else(|| trimmed.strip_prefix('\''))?;
    let content = quoted.strip_suffix('\'')?;
    Some(content.replace("''", "'").replace("\\'", "'"))
}

fn record_module(record: &SqlInsert) -> Option<&'static str> {
    let fields = ["permission", "path", "component", "component_name"]
        .iter()
        .filter_map(|column| record.string_value(column))
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    module_from_route_fields(&fields)
}

fn record_is_demo_menu(record: &SqlInsert) -> bool {
    ["permission", "path", "component", "component_name"]
        .iter()
        .filter_map(|column| record.string_value(column))
        .map(|value| value.to_ascii_lowercase())
        .any(|value| {
            let normalized = value.trim_start_matches('/');
            normalized.starts_with("demo")
                || normalized.contains("/demo")
                || normalized.contains(":demo")
        })
}

fn module_from_route_fields(fields: &[String]) -> Option<&'static str> {
    for (module, prefixes) in module_prefixes() {
        if fields.iter().any(|field| {
            prefixes.iter().any(|prefix| {
                let normalized = field.trim_start_matches('/');
                normalized == *prefix
                    || normalized.starts_with(&format!("{prefix}:"))
                    || normalized.starts_with(&format!("{prefix}/"))
            })
        }) {
            return Some(module);
        }
    }
    None
}

fn module_for_dict_type(dict_type: &str) -> Option<&'static str> {
    let lower = dict_type.to_ascii_lowercase();
    for (module, prefixes) in module_prefixes() {
        if prefixes
            .iter()
            .any(|prefix| lower == *prefix || lower.starts_with(&format!("{prefix}_")))
        {
            return Some(module);
        }
    }
    if lower == "merchant_type" {
        return Some("wms");
    }
    None
}

fn module_for_job_handler(handler: &str) -> Option<&'static str> {
    let lower = handler.to_ascii_lowercase();
    const MALL_JOB_PREFIXES: &[&str] = &[
        "product",
        "promotion",
        "trade",
        "statistics",
        "brokerage",
        "combination",
        "coupon",
    ];
    if MALL_JOB_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return Some("mall");
    }
    module_prefixes().iter().find_map(|(module, prefixes)| {
        prefixes
            .iter()
            .any(|prefix| lower.starts_with(prefix))
            .then_some(*module)
    })
}

fn module_prefixes() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("system", &["system"]),
        ("infra", &["infra"]),
        ("member", &["member"]),
        ("bpm", &["bpm"]),
        ("pay", &["pay"]),
        ("mp", &["mp"]),
        (
            "mall",
            &["mall", "product", "promotion", "trade", "statistics"],
        ),
        ("crm", &["crm"]),
        ("erp", &["erp"]),
        ("iot", &["iot"]),
        ("mes", &["mes"]),
        ("report", &["report"]),
        ("ai", &["ai"]),
        ("wms", &["wms"]),
        ("hrm", &["hrm"]),
        ("fms", &["fms"]),
        ("pms", &["pms"]),
        ("im", &["im"]),
    ]
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

fn rename_backend_project_identifiers(root: &Path, artifact_id: &str) -> Result<(), String> {
    let artifact_id = artifact_id.trim();
    if !is_valid_artifact_id(artifact_id) {
        return Err(format!("非法 Maven artifactId: {artifact_id}"));
    }
    if artifact_id == "yudao" {
        return Ok(());
    }

    let module_prefix = format!("{artifact_id}-");
    rewrite_text_files(root, &[("yudao-", module_prefix.as_str())])?;
    rename_application_entrypoints(root, artifact_id)?;
    rename_prefixed_entries(root, artifact_id)
}

fn is_valid_artifact_id(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some('a'..='z'))
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn upper_camel_artifact_id(artifact_id: &str) -> String {
    artifact_id
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            let Some(first) = characters.next() else {
                return String::new();
            };
            format!(
                "{}{}",
                first.to_ascii_uppercase(),
                characters.collect::<String>()
            )
        })
        .collect()
}

fn rename_application_entrypoints(root: &Path, artifact_id: &str) -> Result<(), String> {
    let mut entrypoints = Vec::new();
    collect_application_entrypoints(root, &mut entrypoints)?;
    let class_prefix = upper_camel_artifact_id(artifact_id);
    let renames = entrypoints
        .into_iter()
        .map(|path| {
            let old_name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("非法启动类文件名: {}", path.display()))?
                .to_string();
            let suffix = old_name
                .strip_prefix("Yudao")
                .ok_or_else(|| format!("无法识别启动类: {}", path.display()))?;
            let new_name = format!("{class_prefix}{suffix}");
            Ok((path, old_name, new_name))
        })
        .collect::<Result<Vec<_>, String>>()?;
    {
        let replacements = renames
            .iter()
            .map(|(_, old_name, new_name)| (old_name.as_str(), new_name.as_str()))
            .collect::<Vec<_>>();
        rewrite_text_files(root, &replacements)?;
    }
    for (path, _, new_name) in renames {
        let target = path.with_file_name(format!("{new_name}.java"));
        if target.exists() {
            return Err(format!("目标启动类已存在: {}", target.display()));
        }
        fs::rename(&path, &target).map_err(|e| {
            format!(
                "重命名启动类 {} -> {} 失败: {e}",
                path.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn collect_application_entrypoints(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| format!("读取启动类目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取启动类目录项失败: {e}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !matches!(name.as_ref(), ".git" | ".idea" | "node_modules" | "target") {
                collect_application_entrypoints(&path, out)?;
            }
        } else if name.starts_with("Yudao") && name.ends_with("Application.java") {
            out.push(path);
        }
    }
    Ok(())
}

fn rename_prefixed_entries(root: &Path, artifact_id: &str) -> Result<(), String> {
    let entries = fs::read_dir(root)
        .map_err(|e| format!("读取待重命名目录失败: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取待重命名目录项失败: {e}"))?;
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() && !matches!(name.as_ref(), ".git" | ".idea" | "node_modules" | "target") {
            rename_prefixed_entries(&path, artifact_id)?;
        }
        if !name.contains("yudao-") {
            continue;
        }
        let renamed = name.replace("yudao-", &format!("{artifact_id}-"));
        let target = path.with_file_name(renamed);
        if target.exists() {
            return Err(format!("目标模块路径已存在: {}", target.display()));
        }
        fs::rename(&path, &target).map_err(|e| {
            format!(
                "重命名模块路径 {} -> {} 失败: {e}",
                path.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn extend_generated_gitignore(root: &Path) -> Result<(), String> {
    let path = root.join(".gitignore");
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("读取生成项目 .gitignore 失败 {}: {e}", path.display()))?;
    let existing = text.lines().map(str::trim).collect::<HashSet<_>>();
    let required = [
        "dependency-reduced-pom.xml",
        "release.properties",
        "pom.xml.releaseBackup",
        ".mvn/timing.properties",
        ".vscode/",
        ".fleet/",
        ".history/",
        "*.code-workspace",
        "*.class",
        "*.tmp",
        "*.temp",
        "*.bak",
        "*.orig",
        "*.rej",
        "*.pid",
        "*.hprof",
        "hs_err_pid*",
        "replay_pid*",
        ".attach_pid*",
        "logs/",
        "log/",
        ".env",
        ".env.*",
        "!.env.example",
    ];
    let missing = required
        .into_iter()
        .filter(|rule| !existing.contains(rule))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    let mut updated = text;
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("\n# Generated project local artifacts\n");
    for rule in missing {
        updated.push_str(rule);
        updated.push('\n');
    }
    fs::write(&path, updated)
        .map_err(|e| format!("写入生成项目 .gitignore 失败 {}: {e}", path.display()))
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

fn write_project_readme(backend_dir: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    let optional_modules = answers
        .modules
        .iter()
        .filter(|module| !matches!(module.as_str(), "system" | "infra"))
        .cloned()
        .collect::<Vec<_>>();
    let sql_note = if optional_modules.is_empty() {
        "".to_string()
    } else {
        format!(
            "\n## 数据库说明\n\n官方开源模板不附带可选模块 `{}` 的完整生产 SQL，请按各模块官方文档另行获取并导入。\n",
            optional_modules.join("`, `")
        )
    };
    let port = if answers.backend == "microservice" {
        answers.gateway_port
    } else {
        answers.monolith_port
    }
    .unwrap_or(DEFAULT_MONOLITH_PORT);
    let entry_module = format!(
        "{}-{}",
        answers.artifact_id,
        if answers.backend == "microservice" {
            "gateway"
        } else {
            "server"
        }
    );
    let readme = format!(
        "# {}\n\n`{}` 是由项目脚手架生成的{}后端项目。\n\n## 项目信息\n\n- Maven：`{}:{}:{}`\n- Java 包：`{}`\n- JDK：{}\n- 业务模块：`{}`\n- 服务端口：{}\n- 多租户：{}\n\n## 本地启动\n\n1. 按需调整 `{}/src/main/resources/application-local.yaml` 中的数据库和 Redis。\n2. 导入 `sql/mysql/ruoyi-vue-pro.sql`（或对应数据库方言）。\n3. 执行 `mvn -pl {} -am spring-boot:run`。\n{}",
        answers.display_name,
        answers.project_name,
        if answers.backend == "microservice" { "微服务" } else { "单体" },
        answers.group_id,
        answers.artifact_id,
        answers.version,
        answers.base_package,
        answers.jdk_version,
        answers.modules.join("`, `"),
        port,
        if answers.tenant_enabled { "启用" } else { "禁用" },
        entry_module,
        entry_module,
        sql_note,
    );
    fs::write(backend_dir.join("README.md"), readme)
        .map_err(|e| format!("写入项目 README 失败: {e}"))
}

fn initialize_git_repository(root: &Path, remotes: &[GitRemote]) -> Result<(), String> {
    if remotes.is_empty() {
        return Ok(());
    }
    let mut names = HashSet::new();
    for remote in remotes {
        let name = remote.name.trim();
        let url = remote.url.trim();
        if !valid_git_remote_name(name) {
            return Err(format!("非法 Git remote 名称: {}", remote.name));
        }
        if url.is_empty() || url.contains(['\r', '\n']) {
            return Err(format!("Git remote {name} 的地址为空或包含换行符"));
        }
        if !names.insert(name.to_string()) {
            return Err(format!("Git remote 名称重复: {name}"));
        }
    }

    let git_dir = root.join(".git");
    if git_dir.exists() {
        return Err(format!("Git 元数据目录已存在: {}", git_dir.display()));
    }
    for relative in ["objects", "refs/heads", "refs/tags"] {
        fs::create_dir_all(git_dir.join(relative))
            .map_err(|e| format!("初始化 Git 目录失败: {e}"))?;
    }
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")
        .map_err(|e| format!("写入 Git HEAD 失败: {e}"))?;

    let mut config = format!(
        "[core]\n\trepositoryformatversion = 0\n\tfilemode = {}\n\tbare = false\n\tlogallrefupdates = true\n",
        if cfg!(windows) { "false" } else { "true" }
    );
    for remote in remotes {
        let name = remote.name.trim();
        config.push_str(&format!(
            "[remote \"{}\"]\n\turl = \"{}\"\n\tfetch = +refs/heads/*:refs/remotes/{}/*\n",
            git_config_escape(name),
            git_config_escape(remote.url.trim()),
            name
        ));
    }
    fs::write(git_dir.join("config"), config).map_err(|e| format!("写入 Git remote 配置失败: {e}"))
}

fn valid_git_remote_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().enumerate().all(|(index, character)| {
            character.is_ascii_alphanumeric() || (index > 0 && matches!(character, '.' | '_' | '-'))
        })
}

fn git_config_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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
            &["system", "bpm"],
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
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.contains("/META-INF/services/") || normalized.starts_with("META-INF/services/") {
        return true;
    }
    if path.extension().is_none() {
        return true;
    }
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
            | "factories"
            | "imports"
            | "sh"
            | "cmd"
            | "bat"
            | "ps1"
            | "vm"
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
    use std::process::Stdio;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
            "yudao:\n  ai:\n    enabled: true\nother: value\nyudao:\n  tenant: # tenant settings\n    enable: true # current\n",
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
    fn keeps_root_and_dependency_bom_revision_in_sync() {
        let test_root = unique_test_dir();
        fs::create_dir_all(test_root.join("yudao-dependencies")).unwrap();
        fs::write(
            test_root.join("pom.xml"),
            "<properties><revision>old-root</revision></properties>\n",
        )
        .unwrap();
        fs::write(
            test_root.join("yudao-dependencies").join("pom.xml"),
            "<properties><revision>old-bom</revision></properties>\n",
        )
        .unwrap();

        set_maven_revision_properties(&test_root, "0.0.1").unwrap();

        assert!(fs::read_to_string(test_root.join("pom.xml"))
            .unwrap()
            .contains("<revision>0.0.1</revision>"));
        assert!(
            fs::read_to_string(test_root.join("yudao-dependencies").join("pom.xml"))
                .unwrap()
                .contains("<revision>0.0.1</revision>")
        );
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
    fn renames_generated_maven_modules_entrypoint_and_templates() {
        let test_root = unique_test_dir();
        let server_java = test_root.join("yudao-server/src/main/java/com/example/server");
        let gateway_java = test_root.join("yudao-gateway/src/main/java/com/example/gateway");
        let velocity = test_root.join("yudao-module-infra/src/main/resources/codegen");
        fs::create_dir_all(&server_java).unwrap();
        fs::create_dir_all(&gateway_java).unwrap();
        fs::create_dir_all(&velocity).unwrap();
        fs::create_dir_all(test_root.join("yudao-dependencies")).unwrap();
        fs::create_dir_all(test_root.join("yudao-framework/yudao-common")).unwrap();
        fs::create_dir_all(test_root.join("yudao-module-system")).unwrap();
        fs::write(
            test_root.join("pom.xml"),
            "<module>yudao-dependencies</module>\n<module>yudao-framework</module>\n<module>yudao-server</module>\n<module>yudao-gateway</module>\n<module>yudao-module-system</module>\n<module>yudao-module-infra</module>\n",
        )
        .unwrap();
        fs::write(
            gateway_java.join("YudaoGatewayApplication.java"),
            "public class YudaoGatewayApplication {}\n",
        )
        .unwrap();
        fs::write(
            server_java.join("YudaoServerApplication.java"),
            "public class YudaoServerApplication { String module = \"yudao-server\"; }\n",
        )
        .unwrap();
        fs::write(
            velocity.join("h2.vm"),
            "copy to yudao-module-${table.moduleName}\n",
        )
        .unwrap();

        rename_backend_project_identifiers(&test_root, "polar").unwrap();

        for directory in [
            "polar-dependencies",
            "polar-framework/polar-common",
            "polar-module-system",
            "polar-module-infra",
            "polar-server",
            "polar-gateway",
        ] {
            assert!(test_root.join(directory).is_dir(), "missing {directory}");
        }
        let main_class = test_root
            .join("polar-server/src/main/java/com/example/server/PolarServerApplication.java");
        let main_text = fs::read_to_string(main_class).unwrap();
        assert!(main_text.contains("class PolarServerApplication"));
        assert!(main_text.contains("polar-server"));
        assert!(!main_text.contains("YudaoServerApplication"));
        assert!(test_root
            .join("polar-gateway/src/main/java/com/example/gateway/PolarGatewayApplication.java")
            .is_file());
        let pom = fs::read_to_string(test_root.join("pom.xml")).unwrap();
        assert!(pom.contains("<module>polar-dependencies</module>"));
        assert!(pom.contains("<module>polar-module-infra</module>"));
        assert!(!pom.contains("yudao-"));
        let velocity = fs::read_to_string(
            test_root.join("polar-module-infra/src/main/resources/codegen/h2.vm"),
        )
        .unwrap();
        assert!(velocity.contains("polar-module-${table.moduleName}"));
        assert!(!velocity.contains("yudao-"));
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn extends_generated_gitignore_without_duplicate_rules() {
        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        fs::write(test_root.join(".gitignore"), "target/\n.idea\n").unwrap();

        extend_generated_gitignore(&test_root).unwrap();
        extend_generated_gitignore(&test_root).unwrap();

        let gitignore = fs::read_to_string(test_root.join(".gitignore")).unwrap();
        for rule in ["logs/", "*.class", ".vscode/", ".env", "!.env.example"] {
            assert!(gitignore.lines().any(|line| line == rule));
            assert_eq!(gitignore.lines().filter(|line| *line == rule).count(), 1);
        }
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn rewrites_spring_runtime_metadata_and_service_descriptors() {
        let test_root = unique_test_dir();
        let metadata = test_root.join("src/main/resources/META-INF");
        let spring = metadata.join("spring");
        let services = metadata.join("services");
        fs::create_dir_all(&spring).unwrap();
        fs::create_dir_all(&services).unwrap();
        let files = [
            metadata.join("spring.factories"),
            spring.join("org.springframework.boot.autoconfigure.AutoConfiguration.imports"),
            services.join("com.example.FrameworkService"),
            test_root.join("Dockerfile"),
        ];
        for path in &files {
            fs::write(path, "cn.iocoder.yudao.framework.Example\n").unwrap();
        }

        rewrite_text_files(
            &test_root,
            &[("cn.iocoder.yudao", "com.example.application")],
        )
        .unwrap();

        for path in files {
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                text.contains("com.example.application.framework.Example"),
                "{} was not rewritten",
                path.display()
            );
            assert!(!text.contains("cn.iocoder.yudao"));
        }
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn prunes_only_unselected_module_configuration() {
        let test_root = unique_test_dir();
        let resources = test_root.join("server/src/main/resources");
        fs::create_dir_all(&resources).unwrap();
        let yaml = resources.join("application.yaml");
        fs::write(
            &yaml,
            concat!(
                "spring:\n",
                "  cache:\n    type: redis\n",
                "spring:\n",
                "  ai:\n    openai:\n      api-key: test\n",
                "flowable:\n  database-schema-update: true\n",
                "wx:\n  mp:\n    app-id: test\n",
                "yudao:\n",
                "  info:\n    version: 1\n",
                "  pay:\n    order-notify-url: test\n",
                "  demo: true\n",
                "  ai:\n    gemini:\n      enable: false\n",
                "    midjourney:\n      enable: true\n",
                "  # base-url: an optional commented example\n",
                "      base-url: https://example.invalid\n",
                "    suno:\n      enable: true\n",
                "  trade:\n    order:\n      pay-expire-time: 1h\n",
                "  iot:\n    message-bus:\n      type: local\n",
                "  security:\n    permit-all_urls: []\n",
            ),
        )
        .unwrap();

        prune_unselected_module_configs(&test_root, &["system".into(), "infra".into()]).unwrap();

        let configured = fs::read_to_string(yaml).unwrap();
        for removed in [
            "flowable:",
            "  pay:",
            "  demo:",
            "  ai:",
            "  trade:",
            "  iot:",
        ] {
            assert!(!configured.contains(removed), "still contains {removed}");
        }
        assert!(configured.contains("  cache:"));
        assert!(configured.contains("  info:"));
        assert!(configured.contains("  security:"));
        assert!(configured.contains("app-id: ${WX_MP_APP_ID:disabled}"));
        assert!(configured.contains("appid: ${WX_MINIAPP_APP_ID:disabled}"));
        assert!(!configured.contains("app-id: test"));
        assert!(!configured.contains("midjourney:"));
        assert!(!configured.contains("suno:"));
        fs::remove_dir_all(test_root).unwrap();
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
    fn replacing_backend_preserves_everything_else_in_the_selected_parent() {
        let test_root = unique_test_dir();
        let backend = test_root.join("backend");
        fs::create_dir_all(&backend).unwrap();
        fs::write(test_root.join("keep-me.txt"), "parent content").unwrap();
        fs::write(backend.join("old.txt"), "old backend").unwrap();

        let mut staged = StagedOutputDir::new(&backend).unwrap();
        fs::write(staged.path().join("new.txt"), "new backend").unwrap();
        staged.commit(&backend).unwrap();

        assert_eq!(
            fs::read_to_string(test_root.join("keep-me.txt")).unwrap(),
            "parent content"
        );
        assert!(!backend.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(backend.join("new.txt")).unwrap(),
            "new backend"
        );

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

    #[test]
    fn applies_custom_database_redis_and_removes_disabled_slave() {
        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        let yaml = test_root.join("application-local.yaml");
        fs::write(
            &yaml,
            concat!(
                "spring:\n",
                "  datasource:\n",
                "    dynamic:\n",
                "      datasource:\n",
                "        master:\n",
                "          url: jdbc:mysql://127.0.0.1/default\n",
                "          username: root\n",
                "          password: 123456\n",
                "        slave:\n",
                "          lazy: true\n",
                "          url: jdbc:mysql://127.0.0.1/default\n",
                "          username: root\n",
                "          password: 123456\n",
                "  data:\n",
                "    redis:\n",
                "      host: 127.0.0.1\n",
                "      port: 6379\n",
                "      database: 0\n",
                "#      password: dev\n",
            ),
        )
        .unwrap();
        let database = DatabaseSettings {
            enabled: true,
            url: "jdbc:mysql://db:3306/app?useSSL=false".into(),
            username: "app".into(),
            password: "db:#password".into(),
            slave_enabled: false,
            ..DatabaseSettings::default()
        };
        let redis = RedisSettings {
            enabled: true,
            host: "cache.local".into(),
            port: 6380,
            database: 2,
            password: "redis:#password".into(),
        };

        configure_connection_profile(&yaml, &database, &redis).unwrap();

        let configured = fs::read_to_string(yaml).unwrap();
        assert!(configured.contains("url: \"jdbc:mysql://db:3306/app?useSSL=false\""));
        assert!(configured.contains("username: \"app\""));
        assert!(configured.contains("password: \"db:#password\""));
        assert!(!configured.contains("        slave:"));
        assert!(configured.contains("host: \"cache.local\""));
        assert!(configured.contains("port: 6380"));
        assert!(configured.contains("database: 2"));
        assert!(configured.contains("password: \"redis:#password\""));

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn removes_template_slave_when_custom_database_is_disabled() {
        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        let yaml = test_root.join("application-dev.yaml");
        fs::write(
            &yaml,
            concat!(
                "spring:\n",
                "  datasource:\n",
                "    dynamic:\n",
                "      datasource:\n",
                "        master:\n",
                "          url: jdbc:mysql://127.0.0.1/default\n",
                "        slave:\n",
                "          lazy: true\n",
                "          url: jdbc:mysql://127.0.0.1/default\n",
                "  data:\n",
                "    redis:\n",
                "      host: 127.0.0.1\n",
            ),
        )
        .unwrap();

        configure_connection_profile(
            &yaml,
            &DatabaseSettings::default(),
            &RedisSettings::default(),
        )
        .unwrap();

        let configured = fs::read_to_string(yaml).unwrap();
        assert!(configured.contains("        master:"));
        assert!(!configured.contains("        slave:"));
        assert!(configured.contains("  data:"));
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn initializes_multiple_git_remotes_without_a_git_executable() {
        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        initialize_git_repository(&test_root, &[]).unwrap();
        assert!(!test_root.join(".git").exists());
        initialize_git_repository(
            &test_root,
            &[
                GitRemote {
                    name: "origin".into(),
                    url: "https://example.com/owner/repo.git".into(),
                },
                GitRemote {
                    name: "backup".into(),
                    url: "git@example.com:owner/repo.git".into(),
                },
            ],
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(test_root.join(".git").join("HEAD")).unwrap(),
            "ref: refs/heads/main\n"
        );
        let config = fs::read_to_string(test_root.join(".git").join("config")).unwrap();
        assert!(config.contains("[remote \"origin\"]"));
        assert!(config.contains("[remote \"backup\"]"));
        assert!(config.contains("https://example.com/owner/repo.git"));
        assert!(config.contains("git@example.com:owner/repo.git"));
        let git_check = std::process::Command::new("git")
            .arg("-C")
            .arg(&test_root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .unwrap();
        assert!(git_check.status.success());
        assert_eq!(String::from_utf8_lossy(&git_check.stdout).trim(), "true");

        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    #[ignore = "requires YUDAO_SCAFFOLD_REAL_TEMPLATE to point at a local template cache"]
    fn generates_and_audits_real_system_infra_template() {
        let source = std::env::var_os("YUDAO_SCAFFOLD_REAL_TEMPLATE")
            .map(PathBuf::from)
            .expect("YUDAO_SCAFFOLD_REAL_TEMPLATE is required");
        let test_root = unique_test_dir();
        let backend = test_root.join("backend");
        copy_dir_contents(&source, &backend).unwrap();
        clean_backend_template(&backend).unwrap();

        let answers = ScaffoldAnswers {
            project_name: "local-audit".into(),
            display_name: "Local Audit".into(),
            output_dir: test_root.to_string_lossy().into_owned(),
            backend: "monolith".into(),
            jdk_version: "17".into(),
            group_id: "com.local.audit".into(),
            artifact_id: "local-audit".into(),
            version: "9.8.7-SNAPSHOT".into(),
            base_package: "com.local.audit".into(),
            git_remotes: vec![GitRemote {
                name: "origin".into(),
                url: "https://example.com/local-audit.git".into(),
            }],
            modules: vec!["system".into(), "infra".into()],
            frontends: Vec::new(),
            monolith_port: Some(49080),
            gateway_port: Some(49080),
            microservice_ports: HashMap::new(),
            super_admin_username: "local-admin".into(),
            super_admin_password: "local-audit-secret".into(),
            database: DatabaseSettings {
                enabled: true,
                url: "jdbc:mysql://db.local:3306/local_audit?useSSL=false".into(),
                username: "audit_user".into(),
                password: "db:#secret".into(),
                slave_enabled: false,
                ..DatabaseSettings::default()
            },
            redis: RedisSettings {
                enabled: true,
                host: "redis.local".into(),
                port: 6380,
                database: 3,
                password: "redis:#secret".into(),
            },
            pull_existing: true,
            force: Some(false),
            tenant_enabled: false,
            vben_variant: Some("antd".into()),
        };

        prune_backend_modules(&backend, &answers.modules).unwrap();
        let removed_rows = customize_backend_tree(&backend, &answers).unwrap();
        write_project_readme(&backend, &answers).unwrap();
        initialize_git_repository(&backend, &answers.git_remotes).unwrap();
        println!("real template SQL rows removed: {removed_rows}");
        assert!(
            removed_rows > 0,
            "the real template must contain removable optional-module seed rows"
        );

        let remaining_modules = fs::read_dir(&backend)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("local-audit-module-"))
            .collect::<HashSet<_>>();
        assert_eq!(
            remaining_modules,
            HashSet::from([
                "local-audit-module-system".to_string(),
                "local-audit-module-infra".to_string()
            ])
        );
        for unwanted in [
            "yudao-ui",
            "yudao-dependencies",
            "yudao-framework",
            "yudao-server",
            ".gitee",
            ".github",
            ".image",
        ] {
            assert!(!backend.join(unwanted).exists());
        }
        assert!(!backend
            .join("local-audit-module-infra/src/main/java/com/local/audit/module/infra/controller/admin/demo")
            .exists());
        assert!(!test_root.join(".scaffold.json").exists());
        assert!(!test_root.join("README.scaffold.md").exists());
        assert!(fs::read_to_string(backend.join("README.md"))
            .unwrap()
            .starts_with("# Local Audit"));
        assert!(fs::read_to_string(backend.join("README.md"))
            .unwrap()
            .contains("mvn -pl local-audit-server -am spring-boot:run"));
        assert!(!test_root.join(".git").exists());
        assert!(backend.join(".git").join("HEAD").is_file());
        let git_config = fs::read_to_string(backend.join(".git").join("config")).unwrap();
        assert!(git_config.contains("[remote \"origin\"]"));
        assert!(git_config.contains("https://example.com/local-audit.git"));

        let pom = fs::read_to_string(backend.join("pom.xml")).unwrap();
        assert!(pom.contains("<groupId>com.local.audit</groupId>"));
        assert!(pom.contains("<artifactId>local-audit</artifactId>"));
        assert!(pom.contains("<revision>9.8.7-SNAPSHOT</revision>"));
        assert!(pom.contains("<java.version>17</java.version>"));
        assert!(pom.contains("<module>local-audit-server</module>"));
        assert!(pom.contains("<module>local-audit-module-system</module>"));
        let dependencies_pom =
            fs::read_to_string(backend.join("local-audit-dependencies").join("pom.xml")).unwrap();
        assert!(dependencies_pom.contains("<revision>9.8.7-SNAPSHOT</revision>"));

        let application = fs::read_to_string(
            backend
                .join("local-audit-server")
                .join("src")
                .join("main")
                .join("resources")
                .join("application.yaml"),
        )
        .unwrap();
        assert!(application.contains("  tenant: # 多租户相关配置项\n    enable: false"));
        let local_profile = fs::read_to_string(
            backend
                .join("local-audit-server")
                .join("src")
                .join("main")
                .join("resources")
                .join("application-local.yaml"),
        )
        .unwrap();
        assert!(local_profile.contains("server:\n  port: 49080"));
        assert!(
            local_profile.contains("url: \"jdbc:mysql://db.local:3306/local_audit?useSSL=false\"")
        );
        assert!(local_profile.contains("username: \"audit_user\""));
        assert!(local_profile.contains("password: \"db:#secret\""));
        assert!(!local_profile.contains("        slave:"));
        assert!(local_profile.contains("host: \"redis.local\""));
        assert!(local_profile.contains("port: 6380"));
        assert!(local_profile.contains("database: 3"));
        assert!(local_profile.contains("password: \"redis:#secret\""));
        assert!(!local_profile.contains("        slave:"));
        assert!(local_profile.contains("app-id: ${WX_MP_APP_ID:disabled}"));
        assert!(local_profile.contains("appid: ${WX_MINIAPP_APP_ID:disabled}"));
        assert!(!local_profile.contains("  pay:"));
        assert!(!local_profile.contains("  demo:"));

        assert!(!application.contains("\nflowable:"));
        assert!(!application.contains("  ai:"));
        assert!(!application.contains("  trade:"));
        assert!(!application.contains("  iot:"));

        audit_root_sql_has_no_unselected_business_tables(&backend.join("sql"));
        audit_runtime_metadata_has_no_old_package(&backend);
        audit_generated_tree_has_no_template_module_prefix(&backend);
        assert_eq!(
            filter_unselected_module_sql(&backend.join("sql"), &answers.modules).unwrap(),
            0,
            "SQL filtering must remove every recognizable unselected module seed row"
        );
        if std::env::var_os("YUDAO_SCAFFOLD_RUN_MAVEN_PACKAGE").is_some() {
            let maven_command = if cfg!(windows) { "mvn.cmd" } else { "mvn" };
            let build = std::process::Command::new(maven_command)
                .current_dir(&backend)
                .args(["-U", "-DskipTests", "package"])
                .output()
                .unwrap();
            assert!(
                build.status.success(),
                "generated Maven project failed to build\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&build.stdout),
                String::from_utf8_lossy(&build.stderr)
            );
            if std::env::var_os("YUDAO_SCAFFOLD_RUN_STARTUP").is_some() {
                smoke_test_generated_server(&backend, &answers.artifact_id);
            }
        }
        fs::remove_dir_all(test_root).unwrap();
    }

    #[test]
    fn filters_unselected_menu_tree_dictionary_and_job_seed_rows() {
        let test_root = unique_test_dir();
        fs::create_dir_all(&test_root).unwrap();
        let sql = test_root.join("seed.sql");
        fs::write(
            &sql,
            concat!(
                "INSERT INTO system_menu (id, name, permission, parent_id, path, component, component_name) VALUES (1, 'System', '', 0, '/system', NULL, NULL);\n",
                "INSERT INTO system_menu (id, name, permission, parent_id, path, component, component_name) VALUES (10, 'Pay', '', 0, '/pay', NULL, NULL);\n",
                "INSERT INTO system_menu (id, name, permission, parent_id, path, component, component_name) VALUES (11, 'Config', '', 10, 'config', NULL, NULL);\n",
                "INSERT INTO system_role_menu (role_id, menu_id) VALUES (1, 11);\n",
                "INSERT INTO system_menu (id, name, permission, parent_id, path, component, component_name) VALUES (20, 'Demo', '', 0, '/demo', NULL, NULL);\n",
                "INSERT INTO system_menu (id, name, permission, parent_id, path, component, component_name) VALUES (21, 'Demo child', '', 20, 'child', NULL, NULL);\n",
                "INSERT INTO system_role_menu (role_id, menu_id) VALUES (1, 21);\n",
                "INSERT INTO system_dict_type (id, name, type) VALUES (1, 'Pay status', 'pay_status');\n",
                "INSERT INTO system_dict_type (id, name, type) VALUES (2, 'System status', 'system_status');\n",
                "INSERT INTO infra_job (id, name, handler_name) VALUES (1, 'Pay sync', 'paySyncJob');\n",
                "INSERT INTO infra_job (id, name, handler_name) VALUES (2, 'System cleanup', 'systemCleanupJob');\n",
                "INSERT INTO infra_codegen_table (id, table_name) VALUES (30, 'yudao_demo01_contact');\n",
                "INSERT INTO infra_codegen_column (id, table_id, column_name) VALUES (31, 30, 'name');\n",
                "-- ----------------------------\n",
                "-- Table structure for yudao_demo01_contact\n",
                "-- ----------------------------\n",
                "CREATE TABLE yudao_demo01_contact (id bigint);\n",
                "-- ----------------------------\n",
                "-- Records of next_table\n",
                "-- ----------------------------\n",
            ),
        )
        .unwrap();

        let removed =
            filter_unselected_module_sql(&test_root, &["system".into(), "infra".into()]).unwrap();

        assert!(removed >= 10);
        let filtered = fs::read_to_string(sql).unwrap();
        assert!(filtered.contains("VALUES (1, 'System'"));
        assert!(filtered.contains("'system_status'"));
        assert!(filtered.contains("'systemCleanupJob'"));
        assert!(!filtered.contains("VALUES (10, 'Pay'"));
        assert!(!filtered.contains("VALUES (11, 'Config'"));
        assert!(!filtered.contains("'pay_status'"));
        assert!(!filtered.contains("'paySyncJob'"));
        assert!(!filtered.contains("system_role_menu"));
        assert!(!filtered.contains("yudao_demo"));

        fs::remove_dir_all(test_root).unwrap();
    }

    fn audit_root_sql_has_no_unselected_business_tables(root: &Path) {
        const FORBIDDEN_PREFIXES: &[&str] = &[
            "member_",
            "bpm_",
            "pay_",
            "mp_",
            "product_",
            "promotion_",
            "trade_",
            "statistics_",
            "crm_",
            "erp_",
            "iot_",
            "mes_",
            "report_",
            "ai_",
        ];
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                audit_root_sql_has_no_unselected_business_tables(&path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("sql") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                let lower = line.to_ascii_lowercase();
                if !lower.contains("create table") {
                    continue;
                }
                assert!(
                    !lower.contains("yudao_demo"),
                    "{} still contains a demo table: {line}",
                    path.display()
                );
                for prefix in FORBIDDEN_PREFIXES {
                    assert!(
                        !lower.contains(prefix),
                        "{} contains an unselected business table: {line}",
                        path.display()
                    );
                }
            }
        }
    }

    fn audit_runtime_metadata_has_no_old_package(root: &Path) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                audit_runtime_metadata_has_no_old_package(&path);
                continue;
            }
            let normalized = path.to_string_lossy().replace('\\', "/");
            if !normalized.contains("/src/main/resources/META-INF/") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            assert!(
                !text.contains("cn.iocoder.yudao"),
                "Spring runtime metadata still references the old package: {}",
                path.display()
            );
        }
    }

    fn audit_generated_tree_has_no_template_module_prefix(root: &Path) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if path.is_dir() {
                if matches!(name.as_ref(), ".git" | "target") {
                    continue;
                }
                audit_generated_tree_has_no_template_module_prefix(&path);
            }
            assert!(
                !name.contains("yudao-"),
                "generated path still uses the template module prefix: {}",
                path.display()
            );
            if should_rewrite_file(&path) {
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                assert!(
                    !text.contains("yudao-"),
                    "generated text still uses the template module prefix: {}",
                    path.display()
                );
            }
        }
    }

    fn smoke_test_generated_server(backend: &Path, artifact_id: &str) {
        let target = backend.join(format!("{artifact_id}-server")).join("target");
        let jar = fs::read_dir(&target)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("jar")
                    && !path.to_string_lossy().ends_with(".jar.original")
            })
            .expect("generated server jar is missing");
        let mut child = std::process::Command::new("java")
            .arg("-jar")
            .arg(jar)
            .args([
                "--spring.boot.admin.client.enabled=false",
                "--spring.datasource.dynamic.datasource.master.url=jdbc:mysql://127.0.0.1:1/local_audit?connectTimeout=1000&socketTimeout=1000",
                "--spring.data.redis.host=127.0.0.1",
                "--spring.data.redis.port=1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start generated server jar");
        std::thread::sleep(Duration::from_secs(12));
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        let output = child.wait_with_output().unwrap();
        let logs = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let application_name = format!("{}ServerApplication", upper_camel_artifact_id(artifact_id));
        assert!(
            logs.contains(&format!("Starting {application_name}"))
                || logs.contains(&format!("Started {application_name}")),
            "generated server did not reach Spring application startup\n{logs}"
        );
        for forbidden in [
            "Unable to instantiate factory class",
            "ClassNotFoundException: cn.iocoder.yudao",
        ] {
            assert!(
                !logs.contains(forbidden),
                "generated server still has stale Spring metadata: {forbidden}\n{logs}"
            );
        }
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
