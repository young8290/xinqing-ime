//! AI 服务地址与密钥（09 D-19、08 FR-AIG-06）。
//!
//! 结构与仓库里的 `secrets.example.toml` 一致；Hub 把它序列化成 TOML 文本后整份用 DPAPI 加密存为
//! `hub\secrets.bin`（加解密在外壳，见 ADR 0011）。本模块只负责格式、校验、合并与转成 [`GatewayConfig`]，
//! 不碰文件和平台接口。TOML 文本里有明文密钥，调用方必须放在 `Zeroizing` 里，用完即清零。

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

use crate::config::{ApiKey, DEFAULT_MODELS, DEV_BASE, GatewayConfig, JevConfig, LlmConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev: Option<JevSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmSecret>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevSecret {
    pub base_url: String,
    #[serde(default, with = "key_serde", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<ApiKey>,
    /// 缺省 `jev-latest`（FR-AIG-02）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSecret {
    pub base_url: String,
    #[serde(default, with = "key_serde", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<ApiKey>,
    /// 按优先级排列；为空时用 [`DEFAULT_MODELS`]
    #[serde(default)]
    pub models: Vec<String>,
}

/// 界面能看到的样子：地址和模型原样，密钥只露末 4 位（FR-AIG-06、FR-SET-08）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaskedSecrets {
    pub jev: Option<MaskedSide>,
    pub llm: Option<MaskedSide>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaskedSide {
    pub base_url: String,
    /// 例如 `••••a1b2`；没有密钥时为空
    pub key_tail: Option<String>,
    /// Jev 是一个模型，大模型是优先级列表
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretsError {
    #[error("{0} 的地址必须以 http:// 或 https:// 开头")]
    BadUrl(&'static str),
    #[error("{0} 的模型名不能为空")]
    EmptyModel(&'static str),
    #[error("AI 配置无法解析：{0}")]
    Parse(String),
}

impl AiSecrets {
    /// dev 构建没有任何配置时用：与 [`GatewayConfig::dev_from_env`] 相同，指向本机 mock-ai，不需要密钥
    /// （14 第 3.3 节）。
    pub fn dev_from_env() -> Self {
        let var = |k: &str, default: String| std::env::var(k).unwrap_or(default);
        Self {
            jev: Some(JevSecret {
                base_url: var("XQ_JEV_BASE_URL", format!("{DEV_BASE}/v1/systemone")),
                api_key: None,
                model: None,
            }),
            llm: Some(LlmSecret {
                base_url: var("XQ_LLM_BASE_URL", format!("{DEV_BASE}/v1")),
                api_key: None,
                models: Vec::new(),
            }),
        }
    }

    /// 两侧都没配置。
    pub fn is_empty(&self) -> bool {
        self.jev.is_none() && self.llm.is_none()
    }

    /// 解析 `secrets.toml` / 解密后的 `secrets.bin` 内容，并校验。
    pub fn from_toml(text: &str) -> Result<Self, SecretsError> {
        let s: AiSecrets = toml::from_str(text).map_err(|e| SecretsError::Parse(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    /// 序列化为 TOML。结果含明文密钥，所以包在 `Zeroizing` 里返回。
    pub fn to_toml(&self) -> Zeroizing<String> {
        Zeroizing::new(toml::to_string(self).expect("AiSecrets 只含字符串，必定可序列化"))
    }

    pub fn validate(&self) -> Result<(), SecretsError> {
        if let Some(j) = &self.jev {
            check_url("Jev", &j.base_url)?;
            if j.model.as_deref().is_some_and(|m| m.trim().is_empty()) {
                return Err(SecretsError::EmptyModel("Jev"));
            }
        }
        if let Some(l) = &self.llm {
            check_url("大模型", &l.base_url)?;
            if l.models.iter().any(|m| m.trim().is_empty()) {
                return Err(SecretsError::EmptyModel("大模型"));
            }
        }
        Ok(())
    }

    /// 用户在设置页保存：没有重新填写密钥（`None`）的一侧沿用旧密钥，界面因此永远不需要拿到明文密钥。
    /// 某一侧整个为 `None` 表示清除这一侧（之后这一侧走离线降级）。
    pub fn merged_onto(mut self, old: &AiSecrets) -> Self {
        if let (Some(new), Some(old)) = (&mut self.jev, &old.jev)
            && new.api_key.is_none()
        {
            new.api_key = old.api_key.clone();
        }
        if let (Some(new), Some(old)) = (&mut self.llm, &old.llm)
            && new.api_key.is_none()
        {
            new.api_key = old.api_key.clone();
        }
        self
    }

    pub fn masked(&self) -> MaskedSecrets {
        MaskedSecrets {
            jev: self.jev.as_ref().map(|j| MaskedSide {
                base_url: j.base_url.clone(),
                key_tail: j.api_key.as_ref().map(ApiKey::masked),
                models: vec![j.model.clone().unwrap_or_else(|| "jev-latest".into())],
            }),
            llm: self.llm.as_ref().map(|l| MaskedSide {
                base_url: l.base_url.clone(),
                key_tail: l.api_key.as_ref().map(ApiKey::masked),
                models: llm_models(&l.models),
            }),
        }
    }

    /// 转成网关配置；超时、限速等其余参数取产品书默认值。
    pub fn to_config(&self) -> GatewayConfig {
        GatewayConfig {
            jev: self.jev.as_ref().map(|j| {
                let mut c = JevConfig::new(j.base_url.trim());
                c.api_key = j.api_key.clone();
                if let Some(m) = &j.model {
                    c.model = m.trim().to_string();
                }
                c
            }),
            llm: self.llm.as_ref().map(|l| {
                let mut c = LlmConfig::new(l.base_url.trim());
                c.api_key = l.api_key.clone();
                c.models = llm_models(&l.models);
                c
            }),
            caps: Vec::new(),
        }
    }
}

fn llm_models(models: &[String]) -> Vec<String> {
    if models.is_empty() {
        DEFAULT_MODELS.iter().map(|m| m.to_string()).collect()
    } else {
        models.iter().map(|m| m.trim().to_string()).collect()
    }
}

fn check_url(side: &'static str, url: &str) -> Result<(), SecretsError> {
    let u = url.trim();
    let rest = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !host.chars().any(char::is_whitespace) => Ok(()),
        _ => Err(SecretsError::BadUrl(side)),
    }
}

/// 密钥按普通字符串读写，但反序列化后立即进 `Zeroizing`，`Debug` 也不会输出。
mod key_serde {
    use super::*;

    pub fn serialize<S: Serializer>(k: &Option<ApiKey>, s: S) -> Result<S::Ok, S::Error> {
        match k {
            Some(k) => s.serialize_str(k.expose()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<ApiKey>, D::Error> {
        let raw = Option::<String>::deserialize(d)?;
        Ok(raw.filter(|k| !k.trim().is_empty()).map(ApiKey::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[jev]
base_url = "https://jev.example/v1/systemone"
api_key  = "jev-key-1234"
model    = "jev-1.13.0"

[llm]
base_url = "https://llm.example/v1"
api_key  = "sk-llm-abcd"
models   = ["m1", "m2"]
"#;

    #[test]
    fn round_trip_keeps_keys_and_models() {
        let s = AiSecrets::from_toml(EXAMPLE).unwrap();
        let again = AiSecrets::from_toml(&s.to_toml()).unwrap();
        let cfg = again.to_config();
        let jev = cfg.jev.unwrap();
        assert_eq!(jev.model, "jev-1.13.0");
        assert_eq!(jev.api_key.unwrap().masked(), "••••1234");
        let llm = cfg.llm.unwrap();
        assert_eq!(llm.models, ["m1", "m2"]);
        assert_eq!(llm.base_url, "https://llm.example/v1");
    }

    #[test]
    fn repo_example_file_parses() {
        // 仓库里的占位文件要能直接拷成 secrets.toml 改值使用（14 第 4 节）
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../secrets.example.toml"
        ))
        .unwrap();
        // 占位地址里有空格，过不了地址校验，这里只核对结构
        let s: AiSecrets = toml::from_str(&text).unwrap();
        assert!(s.jev.is_some() && s.llm.is_some());
        assert_eq!(s.to_config().llm.unwrap().models.len(), 2);
    }

    #[test]
    fn masked_view_never_contains_the_key() {
        let s = AiSecrets::from_toml(EXAMPLE).unwrap();
        let m = s.masked();
        let shown = format!("{m:?}");
        assert!(!shown.contains("jev-key") && !shown.contains("sk-llm"));
        assert_eq!(m.llm.unwrap().key_tail.as_deref(), Some("••••abcd"));
        assert!(!format!("{s:?}").contains("sk-llm"), "Debug 不得输出密钥");
    }

    #[test]
    fn missing_key_keeps_the_old_one_and_missing_side_clears_it() {
        let old = AiSecrets::from_toml(EXAMPLE).unwrap();
        let new = AiSecrets::from_toml(
            r#"
[llm]
base_url = "https://other.example/v1"
api_key = ""
"#,
        )
        .unwrap()
        .merged_onto(&old);
        assert!(new.jev.is_none(), "没提交 Jev 一侧表示清除");
        let llm = new.llm.unwrap();
        assert_eq!(llm.api_key.unwrap().masked(), "••••abcd", "空密钥沿用旧值");
        assert_eq!(llm.base_url, "https://other.example/v1");
        assert!(llm.models.is_empty());
    }

    #[test]
    fn dev_defaults_match_the_gateway_dev_config() {
        let a = AiSecrets::dev_from_env().to_config();
        let b = GatewayConfig::dev_from_env();
        assert_eq!(a.jev.unwrap().base_url, b.jev.unwrap().base_url);
        let (la, lb) = (a.llm.unwrap(), b.llm.unwrap());
        assert_eq!((la.base_url, la.models), (lb.base_url, lb.models));
        assert!(AiSecrets::default().is_empty() && !AiSecrets::dev_from_env().is_empty());
    }

    #[test]
    fn defaults_apply_when_models_are_left_out() {
        let s = AiSecrets::from_toml("[llm]\nbase_url = \"http://127.0.0.1:18080/v1\"\n").unwrap();
        assert_eq!(s.to_config().llm.unwrap().models, DEFAULT_MODELS);
        assert_eq!(s.masked().llm.unwrap().key_tail, None);
    }

    #[test]
    fn rejects_bad_urls_and_empty_models() {
        assert_eq!(
            AiSecrets::from_toml("[jev]\nbase_url = \"jev.example\"\n").unwrap_err(),
            SecretsError::BadUrl("Jev")
        );
        assert_eq!(
            AiSecrets::from_toml("[llm]\nbase_url = \"https:// x\"\n").unwrap_err(),
            SecretsError::BadUrl("大模型")
        );
        assert_eq!(
            AiSecrets::from_toml("[llm]\nbase_url = \"https://x\"\nmodels = [\" \"]\n")
                .unwrap_err(),
            SecretsError::EmptyModel("大模型")
        );
        assert!(matches!(
            AiSecrets::from_toml("[llm\n").unwrap_err(),
            SecretsError::Parse(_)
        ));
    }
}
