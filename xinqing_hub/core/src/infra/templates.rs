//! 加载并校验出厂模板（10 第 7 节、15）。
//!
//! 危机词表与禁用词表只用出厂版本（15 第 5 节），其余模板允许用户目录同名覆盖。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("{file} 校验失败：{reason}")]
    Invalid {
        file: &'static str,
        reason: &'static str,
    },
    #[error("读取 {path} 失败：{source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("解析 {path} 失败：{source}")]
    Toml {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{file} 中的正则无法编译：{pattern}（{source}）")]
    Regex {
        file: &'static str,
        pattern: String,
        source: regex::Error,
    },
}

/// 模板目录：出厂目录 + 可选的用户覆盖目录。
#[derive(Debug, Clone)]
pub struct TemplateDirs {
    pub factory: PathBuf,
    pub user: Option<PathBuf>,
}

impl TemplateDirs {
    pub fn factory_only(dir: impl Into<PathBuf>) -> Self {
        Self {
            factory: dir.into(),
            user: None,
        }
    }

    /// 允许覆盖的模板：用户目录有同名文件时整体替换出厂版本。
    pub fn resolve(&self, name: &str) -> PathBuf {
        if let Some(user) = &self.user {
            let p = user.join(name);
            if p.is_file() {
                return p;
            }
        }
        self.factory.join(name)
    }

    /// 只认出厂版本的模板（危机词表、禁用词表）。
    pub fn factory_path(&self, name: &str) -> PathBuf {
        self.factory.join(name)
    }

    /// 加载允许覆盖的模板；用户文件读取、解析或内容校验失败时回退出厂版本。
    /// 日志只记固定文件名，不输出用户正文或解析错误中的原文。
    pub fn load_with_fallback<T>(
        &self,
        name: &'static str,
        load: impl Fn(&Path) -> Result<T, TemplateError>,
    ) -> Result<T, TemplateError> {
        let factory = self.factory_path(name);
        let selected = self.resolve(name);
        if selected != factory {
            match load(&selected) {
                Ok(value) => return Ok(value),
                Err(_) => eprintln!("用户模板 {name} 无法使用，改用出厂版本"),
            }
        }
        load(&factory)
    }
}

pub fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, TemplateError> {
    let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| TemplateError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

/// 非安全提示词整体覆盖，严格检查版本与正文占位符；失败逐文件回退。
pub fn load_prompt(
    dirs: &TemplateDirs,
    file: &'static str,
    placeholders: &[&str],
) -> Result<(u32, String), TemplateError> {
    dirs.load_with_fallback(file, |path| {
        let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Io {
            path: path.to_path_buf(), source,
        })?;
        let version = text.lines().next()
            .and_then(|line| line.trim().strip_prefix("<!-- version:"))
            .and_then(|line| line.strip_suffix("-->"))
            .and_then(|line| line.trim().parse::<u32>().ok())
            .filter(|version| *version > 0);
        let body = text.lines().filter(|line| !line.trim_start().starts_with("<!--"))
            .collect::<Vec<_>>().join("\n");
        if version.is_none() || body.trim().is_empty()
            || placeholders.iter().any(|key| !body.contains(key)) {
            return Err(TemplateError::Invalid { file, reason: "缺少正整数版本号、正文或必需占位符" });
        }
        Ok((version.unwrap(), body))
    })
}

/// `baseline_default.toml` 中一个特征的人群默认值。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct MedMad {
    pub med: f64,
    pub mad: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BaselineDefault {
    pub version: u32,
    #[serde(default)]
    pub calibrated: bool,
    pub day: HashMap<String, MedMad>,
    pub night: HashMap<String, MedMad>,
}

impl BaselineDefault {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        read_toml(&dirs.resolve("baseline_default.toml"))
    }
}

/// `app_categories.toml`：进程名 → 类别（忽略大小写，支持 `*` 通配）。
#[derive(Debug, Clone, Default)]
pub struct AppCategories {
    rules: Vec<(String, AppCat)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCat {
    Chat,
    Doc,
    Code,
    Browser,
    Other,
}

impl AppCat {
    pub fn as_str(self) -> &'static str {
        match self {
            AppCat::Chat => "chat",
            AppCat::Doc => "doc",
            AppCat::Code => "code",
            AppCat::Browser => "browser",
            AppCat::Other => "other",
        }
    }
}

impl AppCategories {
    pub fn load(dirs: &TemplateDirs) -> Result<Self, TemplateError> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            chat: Vec<String>,
            #[serde(default)]
            doc: Vec<String>,
            #[serde(default)]
            code: Vec<String>,
            #[serde(default)]
            browser: Vec<String>,
        }
        let raw: Raw = read_toml(&dirs.resolve("app_categories.toml"))?;
        let mut rules = Vec::new();
        for (list, cat) in [
            (raw.chat, AppCat::Chat),
            (raw.doc, AppCat::Doc),
            (raw.code, AppCat::Code),
            (raw.browser, AppCat::Browser),
        ] {
            rules.extend(list.into_iter().map(|p| (p.to_lowercase(), cat)));
        }
        Ok(Self { rules })
    }

    pub fn classify(&self, process: &str) -> AppCat {
        let name = process.to_lowercase();
        self.rules
            .iter()
            .find(|(pat, _)| wildcard_match(pat, &name))
            .map(|(_, c)| *c)
            .unwrap_or(AppCat::Other)
    }
}

/// `*` 通配匹配（调用方负责统一大小写）。与 FR-SEN-05 黑名单使用同一语义。
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard() {
        assert!(wildcard_match("*bank*", "icbcbankclient.exe"));
        assert!(wildcard_match("keepass*.exe", "keepassxc.exe"));
        assert!(!wildcard_match("keepass*.exe", "notepad.exe"));
        assert!(wildcard_match("mstsc.exe", "mstsc.exe"));
        assert!(!wildcard_match("mstsc.exe", "mstsc.exe2"));
    }
}
