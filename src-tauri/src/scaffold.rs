use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
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
    sql_filter: bool,
    monolith_port: Option<u16>,
    gateway_port: Option<u16>,
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
    if output_dir.exists() {
        if answers.force != Some(true) {
            return Err("输出目录已存在，请确认强制覆盖后再生成".to_string());
        }
        guard_removable_output_dir(&output_dir)?;
        fs::remove_dir_all(&output_dir).map_err(|e| format!("删除输出目录失败: {e}"))?;
    }
    fs::create_dir_all(&output_dir).map_err(|e| format!("创建输出目录失败: {e}"))?;

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
        .user_agent("yudao-scaffold-ui/0.1")
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
        &paths,
        mirror,
        &payload.url_overrides,
        answers.pull_existing,
        &app,
    )
    .await?;
    let backend_dst = output_dir.join("backend");
    copy_dir_contents(&backend_src, &backend_dst).map_err(|e| format!("复制后端模板失败: {e}"))?;
    customize_tree(&backend_dst, answers)?;
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
            &paths,
            mirror,
            &payload.url_overrides,
            answers.pull_existing,
            &app,
        )
        .await?;
        let dst = output_dir.join("frontend").join(frontend.role_suffix);
        copy_dir_contents(&src, &dst).map_err(|e| format!("复制前端模板失败: {e}"))?;
        customize_tree(&dst, answers)?;
        emit_ok(
            &app,
            &format!(
                "前端模板 {} 已写入 frontend/{}/",
                frontend.template, frontend.role_suffix
            ),
        );
    }

    emit_phase(&app, total - 1, total, "写入脚手架说明");
    write_scaffold_manifest(&output_dir, &payload)?;

    emit_phase(&app, total, total, "生成完成");
    emit_done(&app, &path_string(&output_dir));
    Ok(0)
}

async fn prepare_template(
    client: &reqwest::Client,
    template: TemplateSource,
    paths: &RuntimePaths,
    mirror: &str,
    overrides: &HashMap<String, String>,
    use_cache: bool,
    app: &AppHandle,
) -> Result<PathBuf, String> {
    let local_path = paths.workspace.join(template.name);
    if local_path.exists() {
        emit_info(app, &format!("使用本地模板: {}", local_path.display()));
        return Ok(local_path);
    }

    let cache_path = paths.cache_dir.join(template.name);
    if use_cache && cache_path.exists() {
        emit_info(app, &format!("使用缓存模板: {}", cache_path.display()));
        return Ok(cache_path);
    }

    let source_url = overrides
        .get(template.name)
        .map(String::as_str)
        .unwrap_or_else(|| {
            if mirror == "github" {
                template.github
            } else {
                template.gitee
            }
        });
    let candidates = archive_url_candidates(source_url);
    if candidates.is_empty() {
        return Err(format!("无法从模板地址生成下载链接: {source_url}"));
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

    let mut last_error = String::new();
    for url in candidates {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp
                    .bytes()
                    .await
                    .map_err(|e| format!("读取模板响应失败: {e}"))?;
                tokio::fs::write(&zip_path, &bytes)
                    .await
                    .map_err(|e| format!("写入模板压缩包失败: {e}"))?;
                let zip_path_clone = zip_path.clone();
                let destination_clone = destination.to_path_buf();
                tokio::task::spawn_blocking(move || {
                    extract_zip_strip_root(&zip_path_clone, &destination_clone)
                })
                .await
                .map_err(|e| format!("解压任务失败: {e}"))??;
                let _ = fs::remove_file(&zip_path);
                return Ok(());
            }
            Ok(resp) => {
                last_error = format!("{url} 返回 HTTP {}", resp.status());
            }
            Err(e) => {
                last_error = format!("{url} 下载失败: {e}");
            }
        }
    }

    Err(format!("模板下载失败: {last_error}"))
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

fn customize_tree(root: &Path, answers: &ScaffoldAnswers) -> Result<(), String> {
    let slash_package = answers.base_package.replace('.', "/");
    let backslash_package = answers.base_package.replace('.', "\\");
    let replacements = [
        ("cn.iocoder.yudao", answers.base_package.as_str()),
        ("cn/iocoder/yudao", slash_package.as_str()),
        ("cn\\iocoder\\yudao", backslash_package.as_str()),
        ("1.0.0-snapshot", answers.version.as_str()),
        ("1.0.0-SNAPSHOT", answers.version.as_str()),
        ("ruoyi-vue-pro", answers.project_name.as_str()),
        ("yudao-cloud", answers.project_name.as_str()),
    ];
    rewrite_text_files(root, &replacements)?;
    relocate_java_packages(root, &answers.base_package)?;
    Ok(())
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
        copy_dir_contents(&old, &new).map_err(|e| format!("迁移 Java 包目录失败: {e}"))?;
        let old_cn = java_root.join("cn");
        if old_cn.exists() {
            fs::remove_dir_all(old_cn).map_err(|e| format!("清理旧 Java 包目录失败: {e}"))?;
        }
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
        "sqlFilter": answers.sql_filter,
        "tenantEnabled": answers.tenant_enabled,
        "superAdminUsername": &answers.super_admin_username,
        "superAdminPassword": &answers.super_admin_password,
        "monolithPort": answers.monolith_port,
        "gatewayPort": answers.gateway_port,
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
            &[48083],
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
            &[48084],
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
            &[48087, 48088, 48089, 48090],
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
            &[48091],
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
            &[48092],
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
            &[48093],
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
            &[48094],
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
            &[48095],
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
            &[48096],
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
            json!({
                "name": template.name,
                "kind": template.kind,
                "localPath": path_string(&local),
                "localPresent": local.exists(),
                "cachePath": path_string(&cache),
                "cachePresent": cache.exists(),
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

fn archive_url_candidates(url: &str) -> Vec<String> {
    let trimmed = url.trim().trim_end_matches(".git").trim_end_matches('/');
    if trimmed.ends_with(".zip") {
        return vec![trimmed.to_string()];
    }
    if trimmed.contains("github.com/") {
        return ["master", "main"]
            .iter()
            .map(|branch| format!("{trimmed}/archive/refs/heads/{branch}.zip"))
            .collect();
    }
    if trimmed.contains("gitee.com/") {
        return ["master", "main"]
            .iter()
            .map(|branch| format!("{trimmed}/repository/archive/{branch}.zip"))
            .collect();
    }
    Vec::new()
}

fn should_rewrite_file(path: &Path) -> bool {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "java" | "kt" | "xml" | "yml" | "yaml" | "properties" | "md" | "json" | "ts" | "js"
        | "vue" | "html" | "sql" | "env" | "txt" => true,
        _ => false,
    }
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
