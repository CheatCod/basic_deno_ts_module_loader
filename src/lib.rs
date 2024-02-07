use deno_core::futures::FutureExt;
use deno_core::{resolve_import, ModuleLoader};
use deno_core::{ModuleLoadResponse, RequestedModuleType};

use anyhow::bail;
use deno_ast::MediaType;
use deno_ast::ParseParams;
use deno_ast::SourceTextInfo;
use deno_core::FastString;
use deno_core::ModuleSource;
use deno_core::ModuleSourceCode;
use deno_core::ModuleType;
use deno_core::{anyhow, error::generic_error};

pub struct TypescriptModuleLoader {
    http: reqwest::Client,
}

impl Default for TypescriptModuleLoader {
    fn default() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

impl TypescriptModuleLoader {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

impl ModuleLoader for TypescriptModuleLoader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        _kind: deno_core::ResolutionKind,
    ) -> Result<deno_core::ModuleSpecifier, anyhow::Error> {
        Ok(resolve_import(specifier, referrer)?)
    }

    fn load(
        &self,
        module_specifier: &deno_core::ModuleSpecifier,
        _maybe_referrer: Option<&deno_core::ModuleSpecifier>,
        _is_dyn_import: bool,
        requested_module_type: RequestedModuleType,
    ) -> ModuleLoadResponse {
        let module_specifier = module_specifier.clone();
        let http = self.http.clone();
        let future = async move {
            let (code, module_type, media_type, should_transpile) = match module_specifier
                .to_file_path()
            {
                Ok(path) => {
                    let media_type = MediaType::from_path(&path);

                    let (module_type, should_transpile) = match media_type {
                        MediaType::JavaScript | MediaType::Mjs | MediaType::Cjs => {
                            (ModuleType::JavaScript, false)
                        }
                        MediaType::Jsx => (ModuleType::JavaScript, true),
                        MediaType::TypeScript
                        | MediaType::Mts
                        | MediaType::Cts
                        | MediaType::Dts
                        | MediaType::Dmts
                        | MediaType::Dcts
                        | MediaType::Tsx => (ModuleType::JavaScript, true),
                        MediaType::Json => (ModuleType::Json, false),
                        _ => bail!("Unknown extension {:?}", path.extension()),
                    };

                    if module_type == ModuleType::Json
                        && requested_module_type != RequestedModuleType::Json
                    {
                        return Err(generic_error("Attempted to load JSON module without specifying \"type\": \"json\" attribute in the import statement."));
                    }

                    (
                        tokio::fs::read_to_string(&path).await?,
                        module_type,
                        media_type,
                        should_transpile,
                    )
                }

                Err(_) => {
                    if module_specifier.scheme() == "http" || module_specifier.scheme() == "https" {
                        let http_res = http.get(module_specifier.to_string()).send().await?;

                        if !http_res.status().is_success() {
                            bail!("Failed to fetch module: {module_specifier}");
                        }

                        let content_type = http_res
                            .headers()
                            .get("content-type")
                            .and_then(|ct| ct.to_str().ok())
                            .ok_or_else(|| generic_error("No content-type header"))?;

                        let media_type =
                            MediaType::from_content_type(&module_specifier, content_type);

                        let (module_type, should_transpile) = match media_type {
                            MediaType::JavaScript | MediaType::Mjs | MediaType::Cjs => {
                                (ModuleType::JavaScript, false)
                            }
                            MediaType::Jsx => (ModuleType::JavaScript, true),
                            MediaType::TypeScript
                            | MediaType::Mts
                            | MediaType::Cts
                            | MediaType::Dts
                            | MediaType::Dmts
                            | MediaType::Dcts
                            | MediaType::Tsx => (ModuleType::JavaScript, true),
                            MediaType::Json => (ModuleType::Json, false),
                            _ => bail!("Unknown content-type {:?}", content_type),
                        };

                        if module_type == ModuleType::Json
                            && requested_module_type != RequestedModuleType::Json
                        {
                            return Err(generic_error("Attempted to load JSON module without specifying \"type\": \"json\" attribute in the import statement."));
                        }

                        let code = http_res.text().await?;

                        (code, module_type, media_type, should_transpile)
                    } else {
                        bail!("Unsupported module specifier: {}", module_specifier);
                    }
                }
            };

            let code = if should_transpile {
                let parsed = deno_ast::parse_module(ParseParams {
                    specifier: module_specifier.to_string(),
                    text_info: SourceTextInfo::from_string(code),
                    media_type,
                    capture_tokens: false,
                    scope_analysis: false,
                    maybe_syntax: None,
                })?;

                parsed.transpile(&Default::default())?.text.into_boxed_str()
            } else {
                code.into_boxed_str()
            };

            let module = ModuleSource::new(
                module_type,
                ModuleSourceCode::String(FastString::Owned(code)),
                &module_specifier,
            );

            Ok(module)
        }
        .boxed_local();

        ModuleLoadResponse::Async(future)
    }
}
