use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{
    header::{HeaderValue, CONTENT_TYPE},
    Method, Request, Response, StatusCode,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::{
        streamable_http_server::{
            session::{
                local::{LocalSessionManager, SessionConfig},
                SessionId,
            },
            StreamableHttpServerConfig,
        },
        StreamableHttpService,
    },
    ErrorData, ServerHandler, ServiceExt,
};
use serde_json::json;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use url::Url;

use crate::config::model::AppConfig;
use crate::mcp::context::McpCallContext;
use crate::mcp::edt_syntax;
use crate::mcp::error::{McpInternalError, McpServiceResult};
use crate::mcp::port::{DefaultMcpUseCasePort, McpUseCasePort};
use crate::mcp::request::{
    McpBuildProjectRequest, McpCheckSyntaxDesignerConfigRequest,
    McpCheckSyntaxDesignerModulesRequest, McpCheckSyntaxEdtRequest, McpDumpConfigRequest,
    McpLaunchAppRequest, McpRunAllTestsRequest, McpRunModuleTestsRequest,
};
use crate::mcp::service::McpService;
use crate::mcp::service::{map_syntax_use_case_result, normalize_check_syntax_edt_request};
use crate::mcp::telemetry::{
    McpEdtSessionObserver, McpTelemetry, SemaphoreWaitErrorKind, SemaphoreWaitOutcome,
};
use crate::support::authority::{host_of_authority, host_of_url, Host};

use crate::platform::edt_session::{
    EdtSessionHostOptions, EdtSessionManager, EdtSessionShutdownError,
};

type SharedMcpUseCasePort = Arc<dyn McpUseCasePort + Send + Sync>;
const HTTP_BODY_LIMIT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpTool {
    RunAllTests,
    RunModuleTests,
    BuildProject,
    DumpConfig,
    LaunchApp,
    CheckSyntaxEdt,
    CheckSyntaxDesignerConfig,
    CheckSyntaxDesignerModules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionStage {
    Queued,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorReason {
    Cancelled,
    Timeout,
    JoinFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExecutionPolicy {
    timeout: Option<Duration>,
}

impl ExecutionPolicy {
    const fn bounded(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
        }
    }
}

impl McpTool {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RunAllTests => "run_all_tests",
            Self::RunModuleTests => "run_module_tests",
            Self::BuildProject => "build_project",
            Self::DumpConfig => "dump_config",
            Self::LaunchApp => "launch_app",
            Self::CheckSyntaxEdt => "check_syntax_edt",
            Self::CheckSyntaxDesignerConfig => "check_syntax_designer_config",
            Self::CheckSyntaxDesignerModules => "check_syntax_designer_modules",
        }
    }

    fn execution_policy(self, config: &AppConfig) -> ExecutionPolicy {
        let _ = self;
        ExecutionPolicy::bounded(config.execution_timeout_duration())
    }
}

/// Bootstrap errors returned by MCP transports.
#[derive(Debug, Error)]
pub enum McpServerError {
    #[error("failed to build tokio runtime for MCP transport: {0}")]
    BuildRuntime(std::io::Error),

    #[error("failed to initialize MCP transport: {0}")]
    Bootstrap(String),

    #[error("failed to bind MCP HTTP listener on {address}: {source}")]
    BindHttp {
        address: String,
        source: std::io::Error,
    },

    #[error("failed to start MCP transport: {0}")]
    Start(String),

    #[error("MCP transport task failed: {0}")]
    Task(String),
}

/// Runs the MCP stdio server until the transport closes.
pub fn serve_stdio(config: AppConfig) -> Result<(), McpServerError> {
    let shutdown_timeout = shutdown_grace_period(&config);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("v8tr-mcp")
        .build()
        .map_err(McpServerError::BuildRuntime)?;

    let result = runtime.block_on(async move {
        let server = McpToolServer::stdio(Arc::new(config))?;
        let edt_session = server.edt_session.clone();
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| McpServerError::Start(error.to_string()))?;

        let result = running.waiting().await;
        shutdown_edt_session(edt_session)?;
        result
            .map(|_| ())
            .map_err(|error| McpServerError::Task(error.to_string()))
    });

    runtime.shutdown_timeout(shutdown_timeout);
    result
}

/// Runs the MCP streamable HTTP server until shutdown.
pub fn serve_http(config: AppConfig) -> Result<(), McpServerError> {
    let shutdown_timeout = shutdown_grace_period(&config);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("v8tr-mcp")
        .build()
        .map_err(McpServerError::BuildRuntime)?;

    let result = runtime.block_on(async move {
        let config = Arc::new(config);
        let shutdown = CancellationToken::new();
        let server = McpToolServer::http(config.clone())?;
        let edt_session = server.edt_session.clone();
        let service = HttpMcpService::new(server, config.clone(), shutdown.child_token());
        let listener = tokio::net::TcpListener::bind(config.mcp.http.bind_address.as_str())
            .await
            .map_err(|source| McpServerError::BindHttp {
                address: config.mcp.http.bind_address.clone(),
                source,
            })?;
        warn_about_a_listener_nobody_guards(config.as_ref());
        let router = axum::Router::new()
            .route(
                config.mcp.http.path.as_str(),
                axum::routing::any({
                    let service = service.clone();
                    move |request| {
                        let service = service.clone();
                        async move { service.handle(request).await }
                    }
                }),
            )
            // Слой, а не проверка внутри обработчика: так под проверку попадают и
            // запросы, не совпавшие ни с одним маршрутом. Отрабатывает он раньше,
            // чем тело запроса начнут читать, а сессию — занимать.
            //
            // Вызов обязан быть последним в цепочке: axum накрывает слоем только
            // те маршруты, что добавлены до него, — маршрут, приписанный следом,
            // пройдёт мимо проверки молча.
            .layer(axum::middleware::from_fn_with_state(
                KnownHosts::of(config.as_ref()),
                refuse_a_request_that_names_another_host,
            ));
        let serve = axum::serve(listener, router).with_graceful_shutdown({
            let shutdown = shutdown.clone();
            async move {
                wait_for_shutdown_signal().await;
                shutdown.cancel();
            }
        });

        let result = serve.await;
        drop(service);
        shutdown.cancel();
        shutdown_edt_session(edt_session)?;
        result.map_err(|error| McpServerError::Task(error.to_string()))
    });

    runtime.shutdown_timeout(shutdown_timeout);
    result
}

fn shutdown_edt_session(edt_session: Arc<EdtSessionManager>) -> Result<(), McpServerError> {
    edt_session
        .shutdown()
        .map_err(|error| McpServerError::Task(format_edt_shutdown_error(error)))
}

fn format_edt_shutdown_error(error: EdtSessionShutdownError) -> String {
    error.to_string()
}

/// rmcp-backed MCP transport adapter over the MCP service layer.
#[derive(Clone)]
pub struct McpToolServer {
    config: Arc<AppConfig>,
    port: SharedMcpUseCasePort,
    edt_session: Arc<EdtSessionManager>,
    concurrency_limit: Arc<Semaphore>,
    telemetry: Arc<McpTelemetry>,
    call_context: McpCallContext,
    tool_router: ToolRouter<Self>,
}

impl McpToolServer {
    /// Creates a stdio server using the production use-case port.
    pub fn stdio(config: Arc<AppConfig>) -> Result<Self, McpServerError> {
        Self::with_port(
            config,
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
    }

    /// Creates an HTTP server using the production use-case port.
    pub fn http(config: Arc<AppConfig>) -> Result<Self, McpServerError> {
        Self::with_port(
            config,
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::http(),
        )
    }

    /// Creates a transport-aware server with an injected MCP use-case port.
    pub fn with_port(
        config: Arc<AppConfig>,
        port: SharedMcpUseCasePort,
        call_context: McpCallContext,
    ) -> Result<Self, McpServerError> {
        let telemetry = Arc::new(McpTelemetry::default());
        Ok(Self {
            edt_session: Arc::new(
                EdtSessionManager::for_config_with_observer(
                    config.as_ref(),
                    EdtSessionHostOptions::for_mcp_host(config.as_ref()),
                    Arc::new(McpEdtSessionObserver::new(telemetry.edt())),
                )
                .map_err(|error| McpServerError::Bootstrap(error.to_string()))?,
            ),
            concurrency_limit: Arc::new(Semaphore::new(max_concurrent_calls(config.as_ref()))),
            telemetry,
            config,
            port,
            call_context,
            tool_router: Self::tool_router(),
        })
    }

    async fn execute_tool<TRequest, TResponse>(
        &self,
        tool: McpTool,
        request: TRequest,
        cancellation: CancellationToken,
        method: impl FnOnce(
                Arc<AppConfig>,
                SharedMcpUseCasePort,
                McpCallContext,
                TRequest,
            ) -> McpServiceResult<TResponse>
            + Send
            + 'static,
    ) -> Result<CallToolResult, ErrorData>
    where
        TRequest: Send + 'static,
        TResponse: serde::Serialize + Send + 'static,
    {
        let policy = tool.execution_policy(self.config.as_ref());
        let timeout = policy.timeout;
        let deadline = timeout.map(|value| Instant::now() + value);
        let permit = self
            .acquire_execution_slot(tool, cancellation.clone(), deadline, timeout)
            .await?;
        let remaining_timeout = remaining_timeout(deadline);
        if cancellation.is_cancelled() {
            return Err(execution_error(
                ErrorReason::Cancelled,
                ExecutionStage::Queued,
                timeout,
            ));
        }
        if timeout.is_some() && remaining_timeout.is_some_and(|value| value.is_zero()) {
            return Err(execution_error(
                ErrorReason::Timeout,
                ExecutionStage::Queued,
                timeout,
            ));
        }

        let config = self.config.clone();
        let port = self.port.clone();
        let call_context = self
            .call_context
            .clone()
            .with_deadline(deadline.map(Instant::into_std))
            .with_cancellation(cancellation.clone())
            .with_edt_timeout(remaining_timeout);
        let mut handle =
            tokio::task::spawn_blocking(move || method(config, port, call_context, request));
        let _permit = permit;
        let mut interrupted = None;
        loop {
            tokio::select! {
                biased;
                result = &mut handle => {
                    let result = result
                        .map_err(|_| execution_error(ErrorReason::JoinFailure, ExecutionStage::Running, timeout))?;
                    return map_tool_result(result);
                }
                _ = cancellation.cancelled(), if interrupted.is_none() => {
                    interrupted = Some(ErrorReason::Cancelled);
                }
                _ = wait_for_deadline(deadline), if deadline.is_some() && interrupted.is_none() => {
                    cancellation.cancel();
                    interrupted = Some(ErrorReason::Timeout);
                }
            }
        }
    }

    async fn acquire_execution_slot(
        &self,
        tool: McpTool,
        cancellation: CancellationToken,
        deadline: Option<Instant>,
        timeout: Option<Duration>,
    ) -> Result<OwnedSemaphorePermit, ErrorData> {
        let wait_started = Instant::now();
        let bounded = timeout.is_some();
        let acquire = self.concurrency_limit.clone().acquire_owned();
        tokio::pin!(acquire);

        match deadline {
            Some(deadline) => {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::Cancelled,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            None,
                        );
                        Err(execution_error(ErrorReason::Cancelled, ExecutionStage::Queued, timeout))
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::Timeout,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            None,
                        );
                        Err(execution_error(ErrorReason::Timeout, ExecutionStage::Queued, timeout))
                    }
                    permit = &mut acquire => permit.map_err(|error| {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::InternalError,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            Some(SemaphoreWaitErrorKind::SemaphoreClosed),
                        );
                        ErrorData::internal_error(error.to_string(), None)
                    }).inspect(|_permit| {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::Acquired,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            None,
                        );
                    }),
                }
            }
            None => {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::Cancelled,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            None,
                        );
                        Err(execution_error(ErrorReason::Cancelled, ExecutionStage::Queued, timeout))
                    }
                    permit = &mut acquire => permit.map_err(|error| {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::InternalError,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            Some(SemaphoreWaitErrorKind::SemaphoreClosed),
                        );
                        ErrorData::internal_error(error.to_string(), None)
                    }).inspect(|_permit| {
                        self.telemetry.execution().record_semaphore_wait(
                            self.call_context.transport(),
                            tool.as_str(),
                            SemaphoreWaitOutcome::Acquired,
                            bounded,
                            timeout,
                            wait_started.elapsed(),
                            None,
                        );
                    }),
                }
            }
        }
    }

    async fn execute_edt_syntax_tool(
        &self,
        request: McpCheckSyntaxEdtRequest,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.config.tools.edt_cli.interactive_mode {
            return self
                .execute_tool(
                    McpTool::CheckSyntaxEdt,
                    request,
                    cancellation,
                    |config, port, call_context, request| {
                        let service = McpService::with_port(config.as_ref(), port);
                        service.check_syntax_edt(call_context, &request)
                    },
                )
                .await;
        }

        let timeout = McpTool::CheckSyntaxEdt
            .execution_policy(self.config.as_ref())
            .timeout;
        let deadline = timeout.map(|value| Instant::now() + value);
        let mut permit = Some(
            self.acquire_execution_slot(
                McpTool::CheckSyntaxEdt,
                cancellation.clone(),
                deadline,
                timeout,
            )
            .await?,
        );
        if cancellation.is_cancelled() {
            return Err(execution_error(
                ErrorReason::Cancelled,
                ExecutionStage::Queued,
                timeout,
            ));
        }

        let remaining_timeout = remaining_timeout(deadline);
        if remaining_timeout.is_some_and(|value| value.is_zero()) {
            return Err(execution_error(
                ErrorReason::Timeout,
                ExecutionStage::Queued,
                timeout,
            ));
        }

        // Срок шага — свой у инструмента; остаток бюджета команды только укорачивает его.
        // Отсутствие бюджета означает «предела сверху нет», а не «времени нет»: миллисекунда
        // в этой ветке превращала бы каждый вызов без бюджета в мгновенный отказ.
        let edt_step_timeout = Duration::from_millis(self.config.tools.edt_cli.command_timeout_ms);
        let edt_timeout = remaining_timeout
            .map(|value| value.min(edt_step_timeout))
            .unwrap_or(edt_step_timeout);
        let use_case_request = normalize_check_syntax_edt_request(&request);
        let result = edt_syntax::execute(
            self.edt_session.as_ref(),
            self.config.as_ref(),
            &use_case_request,
            edt_timeout,
            cancellation,
        )
        .await;

        match result {
            Ok(use_case_result) => {
                permit.take();
                map_tool_result(map_syntax_use_case_result(use_case_result))
            }
            Err(edt_syntax::EdtSyntaxTransportError::QueuedCancelled) => {
                permit.take();
                Err(execution_error(
                    ErrorReason::Cancelled,
                    ExecutionStage::Queued,
                    timeout,
                ))
            }
            Err(edt_syntax::EdtSyntaxTransportError::QueuedTimeout) => {
                permit.take();
                Err(execution_error(
                    ErrorReason::Timeout,
                    ExecutionStage::Queued,
                    timeout,
                ))
            }
        }
    }
}

#[derive(Debug, Default)]
struct HttpSessionAdmissionState {
    reserved: usize,
    active_sessions: HashSet<SessionId>,
}

#[derive(Clone)]
struct HttpSessionAdmission {
    max_sessions: usize,
    session_manager: Arc<LocalSessionManager>,
    state: Arc<Mutex<HttpSessionAdmissionState>>,
}

impl HttpSessionAdmission {
    fn new(max_sessions: usize, session_manager: Arc<LocalSessionManager>) -> Self {
        Self {
            max_sessions: max_sessions.max(1),
            session_manager,
            state: Arc::new(Mutex::new(HttpSessionAdmissionState::default())),
        }
    }

    async fn reserve_initialize(&self) -> Result<HttpSessionReservation, HttpOverloadError> {
        let live_sessions = self.session_manager.sessions.read().await;
        let mut state = self
            .state
            .lock()
            .expect("http session admission mutex poisoned");
        state
            .active_sessions
            .retain(|session_id| live_sessions.contains_key(session_id));
        if state.active_sessions.len() + state.reserved >= self.max_sessions {
            return Err(HttpOverloadError);
        }
        state.reserved += 1;
        drop(state);
        drop(live_sessions);

        Ok(HttpSessionReservation {
            admission: self.clone(),
            completed: false,
        })
    }

    fn confirm(&self, session_id: SessionId) {
        let mut state = self
            .state
            .lock()
            .expect("http session admission mutex poisoned");
        state.reserved = state.reserved.saturating_sub(1);
        state.active_sessions.insert(session_id);
    }

    fn release(&self) {
        let mut state = self
            .state
            .lock()
            .expect("http session admission mutex poisoned");
        state.reserved = state.reserved.saturating_sub(1);
    }

    fn remove(&self, session_id: &SessionId) {
        let mut state = self
            .state
            .lock()
            .expect("http session admission mutex poisoned");
        state.active_sessions.remove(session_id);
    }
}

struct HttpSessionReservation {
    admission: HttpSessionAdmission,
    completed: bool,
}

impl HttpSessionReservation {
    fn confirm(mut self, session_id: SessionId) {
        self.admission.confirm(session_id);
        self.completed = true;
    }

    fn release(mut self) {
        self.admission.release();
        self.completed = true;
    }
}

impl Drop for HttpSessionReservation {
    fn drop(&mut self) {
        if !self.completed {
            self.admission.release();
            self.completed = true;
        }
    }
}

#[derive(Debug)]
struct HttpOverloadError;

#[derive(Clone)]
struct HttpMcpService {
    inner: StreamableHttpService<McpToolServer, LocalSessionManager>,
    admission: HttpSessionAdmission,
    stateful_sessions: bool,
}

impl HttpMcpService {
    fn new(server: McpToolServer, config: Arc<AppConfig>, shutdown: CancellationToken) -> Self {
        let session_manager = Arc::new(LocalSessionManager {
            session_config: SessionConfig {
                keep_alive: config
                    .mcp
                    .http
                    .stateful_sessions
                    .then_some(Duration::from_secs(config.mcp.http.idle_ttl_secs.max(1))),
                ..Default::default()
            },
            ..Default::default()
        });
        let admission =
            HttpSessionAdmission::new(config.mcp.http.max_sessions, session_manager.clone());
        let inner = StreamableHttpService::new(
            move || Ok(server.clone()),
            session_manager,
            StreamableHttpServerConfig {
                stateful_mode: config.mcp.http.stateful_sessions,
                cancellation_token: shutdown,
                ..Default::default()
            },
        );

        Self {
            inner,
            admission,
            stateful_sessions: config.mcp.http.stateful_sessions,
        }
    }

    async fn handle(&self, request: Request<Body>) -> Response<Body> {
        let session_id = session_id_from_headers(request.headers());
        let method = request.method().clone();
        let is_initialize_candidate =
            self.stateful_sessions && method == Method::POST && session_id.is_none();

        if !is_initialize_candidate {
            let response = self.inner.handle(request).await.map(Body::new);
            if method == Method::DELETE && response.status() == StatusCode::ACCEPTED {
                if let Some(session_id) = session_id {
                    self.admission.remove(&session_id);
                }
            }
            return response;
        }

        if !valid_streamable_post_headers(request.headers()) {
            return self.inner.handle(request).await.map(Body::new);
        }

        let (request, rpc_method) = match extract_http_rpc_method(request).await {
            Ok(result) => result,
            Err(response) => return response,
        };
        if rpc_method.as_deref() != Some("initialize") {
            let _ = request;
            return missing_initialize_response();
        }

        let reservation = match self.admission.reserve_initialize().await {
            Ok(reservation) => reservation,
            Err(_) => return overload_response(),
        };

        let response = self.inner.handle(request).await.map(Body::new);
        if response.status() == StatusCode::OK {
            if let Some(session_id) = session_id_from_headers(response.headers()) {
                reservation.confirm(session_id);
            } else {
                reservation.release();
            }
        } else {
            reservation.release();
        }

        response
    }
}

#[tool_router(router = tool_router)]
impl McpToolServer {
    #[tool(description = "Run all tests")]
    async fn run_all_tests(
        &self,
        Parameters(request): Parameters<McpRunAllTestsRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::RunAllTests,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.run_all_tests(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Run tests for a specific module")]
    async fn run_module_tests(
        &self,
        Parameters(request): Parameters<McpRunModuleTestsRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::RunModuleTests,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.run_module_tests(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Build the project")]
    async fn build_project(
        &self,
        Parameters(request): Parameters<McpBuildProjectRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::BuildProject,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.build_project(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Dump configuration to files")]
    async fn dump_config(
        &self,
        Parameters(request): Parameters<McpDumpConfigRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::DumpConfig,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.dump_config(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Launch a 1C application")]
    async fn launch_app(
        &self,
        Parameters(request): Parameters<McpLaunchAppRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::LaunchApp,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.launch_app(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Run EDT syntax check")]
    async fn check_syntax_edt(
        &self,
        Parameters(request): Parameters<McpCheckSyntaxEdtRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_edt_syntax_tool(request, cancellation).await
    }

    #[tool(description = "Run Designer configuration syntax check")]
    async fn check_syntax_designer_config(
        &self,
        Parameters(request): Parameters<McpCheckSyntaxDesignerConfigRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::CheckSyntaxDesignerConfig,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.check_syntax_designer_config(call_context, &request)
            },
        )
        .await
    }

    #[tool(description = "Run Designer modules syntax check")]
    async fn check_syntax_designer_modules(
        &self,
        Parameters(request): Parameters<McpCheckSyntaxDesignerModulesRequest>,
        cancellation: CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        self.execute_tool(
            McpTool::CheckSyntaxDesignerModules,
            request,
            cancellation,
            |config, port, call_context, request| {
                let service = McpService::with_port(config.as_ref(), port);
                service.check_syntax_designer_modules(call_context, &request)
            },
        )
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpToolServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

fn internal_error_to_mcp(error: McpInternalError) -> ErrorData {
    ErrorData::internal_error(error.message, None)
}

fn map_tool_result<TResponse>(
    result: McpServiceResult<TResponse>,
) -> Result<CallToolResult, ErrorData>
where
    TResponse: serde::Serialize + Send + 'static,
{
    match result {
        Ok(response) => Ok(CallToolResult::structured(
            serde_json::to_value(response)
                .map_err(|error| ErrorData::internal_error(error.to_string(), None))?,
        )),
        Err(crate::mcp::error::McpServiceError::Business(failure)) => {
            Ok(CallToolResult::structured_error(
                serde_json::to_value(failure.response)
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None))?,
            ))
        }
        Err(crate::mcp::error::McpServiceError::Internal(error)) => {
            Err(internal_error_to_mcp(error))
        }
    }
}

fn execution_error(
    reason: ErrorReason,
    stage: ExecutionStage,
    timeout: Option<Duration>,
) -> ErrorData {
    let message = match (reason, stage) {
        (ErrorReason::Cancelled, ExecutionStage::Queued) => {
            "MCP call cancelled while waiting for execution slot"
        }
        (ErrorReason::Cancelled, ExecutionStage::Running) => "MCP call cancelled during execution",
        (ErrorReason::Timeout, ExecutionStage::Queued) => {
            "MCP call timed out while waiting for execution slot"
        }
        (ErrorReason::Timeout, ExecutionStage::Running) => "MCP call timed out during execution",
        (ErrorReason::JoinFailure, ExecutionStage::Queued) => "MCP queue task failed unexpectedly",
        (ErrorReason::JoinFailure, ExecutionStage::Running) => {
            "MCP execution task failed unexpectedly"
        }
    };
    let data = json!({
        "reason": match reason {
            ErrorReason::Cancelled => "cancelled",
            ErrorReason::Timeout => "timeout",
            ErrorReason::JoinFailure => "join_failure",
        },
        "stage": match stage {
            ExecutionStage::Queued => "queued",
            ExecutionStage::Running => "running",
        },
        "timeoutMs": timeout.map(duration_to_millis),
    });
    ErrorData::internal_error(message, Some(data))
}

fn remaining_timeout(deadline: Option<Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()))
}

fn duration_to_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

async fn extract_http_rpc_method(
    request: Request<Body>,
) -> Result<(Request<Body>, Option<String>), Response<Body>> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, HTTP_BODY_LIMIT_BYTES)
        .await
        .map_err(|error| {
            Response::builder()
                .status(StatusCode::PAYLOAD_TOO_LARGE)
                .header(
                    CONTENT_TYPE,
                    HeaderValue::from_static("text/plain; charset=utf-8"),
                )
                .body(Body::from(format!("Payload Too Large: {error}")))
                .expect("valid overload response")
        })?;
    let method = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        });
    Ok((Request::from_parts(parts, Body::from(body)), method))
}

fn overload_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(
            "Service Unavailable: MCP session capacity exhausted",
        ))
        .expect("valid overload response")
}

fn missing_initialize_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(
            "Bad Request: initialize request is required before session creation",
        ))
        .expect("valid missing initialize response")
}

fn session_id_from_headers(headers: &axum::http::HeaderMap) -> Option<SessionId> {
    headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(Into::into)
}

/// Отказывает запросу, назвавшему чужой хост.
async fn refuse_a_request_that_names_another_host(
    axum::extract::State(known): axum::extract::State<KnownHosts>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response<Body> {
    match admission_verdict(&request, &known) {
        Ok(()) => next.run(request).await,
        Err(refused) => refusal_response(refused),
    }
}

/// Предупреждает, когда слушатель открыт наружу и закрыт только периметром.
///
/// Сочетание видно до первого запроса, а отказ виден только по коду `403`, который
/// клиент MCP покажет без тела. Поэтому о нём говорят на старте, а не молчат до
/// первой неудачи в контейнере, за которым никто не смотрит.
fn warn_about_a_listener_nobody_guards(config: &AppConfig) {
    if !config.mcp.http.allowed_hosts.is_empty() {
        return;
    }
    let reachable_from_outside = config
        .mcp
        .http
        .bind_address
        .parse::<SocketAddr>()
        .is_ok_and(|address| !address.ip().to_canonical().is_loopback());
    if reachable_from_outside {
        tracing::warn!(
            bind_address = %config.mcp.http.bind_address,
            "MCP HTTP listens beyond the loopback while mcp.http.allowed_hosts is empty: \
             every request naming another host is refused with 403, and the listener \
             itself has no authentication"
        );
    }
}

/// Хосты, на чьё имя слушатель отвечает.
///
/// Защита здесь ровно от одного: браузер на той же машине переразрешает своё имя
/// в `127.0.0.1` и стучится к слушателю как к своему. Подделать `Host` он не может,
/// поэтому сверка имени такую страницу и отсекает.
///
/// Чего защита НЕ делает: она не закрывает слушатель от не-браузерных клиентов.
/// `curl -H 'Host: 127.0.0.1:3000' http://10.0.0.5:3000/mcp` заголовок подставит
/// любой, поэтому не-петлевой bind остаётся открытым всем, кто до него дотянется.
/// Единственная защита там — периметр, и назвать чужое имя в `allowed_hosts`
/// значит взять его на себя.
#[derive(Clone)]
struct KnownHosts {
    named: Arc<Vec<Host>>,
}

impl KnownHosts {
    fn of(config: &AppConfig) -> Self {
        Self {
            named: Arc::new(
                config
                    .mcp
                    .http
                    .allowed_hosts
                    .iter()
                    .filter_map(|allowed| host_of_authority(allowed))
                    .collect(),
            ),
        }
    }

    fn admits(&self, host: &Host) -> bool {
        host.is_loopback() || self.named.contains(host)
    }
}

/// Заголовок, из-за которого запрос отклонён.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefusedHeader {
    Host,
    Origin,
}

impl RefusedHeader {
    const fn name(self) -> &'static str {
        match self {
            RefusedHeader::Host => "Host",
            RefusedHeader::Origin => "Origin",
        }
    }
}

/// Пускать ли запрос по его заголовкам.
///
/// `Host` обязателен и должен быть ровно один: сборка собрана только с `http1`
/// (`Cargo.toml`, крейта `h2` в `Cargo.lock` нет), поэтому `:authority` сюда не
/// приходит и отсутствие заголовка — не протокол версии 2, а причина отказать.
///
/// `Origin` проверяется, если он есть. Отсутствие — не повод отказывать: клиенты
/// вне браузера его не шлют. А `Origin: null` — это не отсутствие: так
/// представляются песочница, `file:` и `data:`, и адресом это не разбирается.
fn admission_verdict(request: &Request<Body>, known: &KnownHosts) -> Result<(), RefusedHeader> {
    // У http/1.1 два канала для имени хоста. Цель запроса в absolute-form
    // (`GET http://имя/mcp HTTP/1.1`) несёт своё, и по RFC 9112 §3.2.2 верить
    // положено ему, а не заголовку; hyper его в `Host` не переносит. Клиенты MCP
    // так не пишут, поэтому здесь требуется, чтобы оба канала назвали известное
    // имя, — строже, чем велит RFC, и закрыто в обе стороны.
    if let Some(authority) = request.uri().authority() {
        let named = host_of_authority(authority.as_str()).ok_or(RefusedHeader::Host)?;
        if !known.admits(&named) {
            return Err(RefusedHeader::Host);
        }
    }

    let headers = request.headers();
    let host = exactly_one(headers, &axum::http::header::HOST)
        .and_then(host_of_authority)
        .ok_or(RefusedHeader::Host)?;
    if !known.admits(&host) {
        return Err(RefusedHeader::Host);
    }

    let Some(origin) = exactly_one(headers, &axum::http::header::ORIGIN) else {
        return if headers.contains_key(axum::http::header::ORIGIN) {
            Err(RefusedHeader::Origin)
        } else {
            Ok(())
        };
    };
    let origin = Url::parse(origin).map_err(|_| RefusedHeader::Origin)?;
    // Браузер сериализует источник как `схема://хост[:порт]` и больше ничем. Всё
    // остальное — userinfo, путь, запрос — оттуда прийти не может, и принимать
    // такое значит рассуждать про одни записи, а пускать другие.
    let shaped_like_an_origin = origin.username().is_empty()
        && origin.password().is_none()
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none();
    let scheme_is_web = origin.scheme() == "http" || origin.scheme() == "https";
    let admitted = scheme_is_web
        && shaped_like_an_origin
        && host_of_url(&origin).is_some_and(|host| known.admits(&host));
    if admitted {
        Ok(())
    } else {
        Err(RefusedHeader::Origin)
    }
}

/// Значение заголовка, если он один.
///
/// Повтор `Host` запрещён RFC 9112 §3.2, но hyper его не отбрасывает, а
/// `HeaderMap::get` молча берёт первый. Считать здесь дешевле, чем гадать, какой
/// из двух прочитает следующий в цепочке.
fn exactly_one<'a>(
    headers: &'a axum::http::HeaderMap,
    name: &axum::http::HeaderName,
) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let only = values.next()?;
    if values.next().is_some() {
        return None;
    }
    // Значение вне печатного ASCII читается как отсутствие: имя хоста таким не бывает.
    only.to_str().ok()
}

/// Ответ на запрос, пришедший не на то имя.
///
/// Значение заголовка в тело не попадает: его пишет тот, кому отказали.
fn refusal_response(refused: RefusedHeader) -> Response<Body> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(format!(
            "Forbidden: the {} header does not name a host this listener answers; \
             list it in mcp.http.allowed_hosts to allow it",
            refused.name()
        )))
        .expect("valid refusal response")
}

fn valid_streamable_post_headers(headers: &axum::http::HeaderMap) -> bool {
    let accepts_both = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.contains("application/json") && value.contains("text/event-stream")
        });
    let content_type_is_json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));

    accepts_both && content_type_is_json
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn max_concurrent_calls(config: &AppConfig) -> usize {
    config.mcp.execution.max_concurrent_calls.max(1)
}

fn shutdown_grace_period(config: &AppConfig) -> Duration {
    Duration::from_secs(config.mcp.execution.shutdown_grace_period_secs.max(1))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;

    use super::{
        admission_verdict, execution_error, max_concurrent_calls, shutdown_grace_period,
        ErrorReason, ExecutionStage, HttpSessionAdmission, KnownHosts, McpTool, McpToolServer,
        RefusedHeader,
    };
    use axum::body::Body;
    use axum::http::Request;

    use crate::config::model::{
        AppConfig, BuildConfig, McpConfig, McpExecutionConfig, McpHttpConfig, PlatformToolConfig,
        SourceFormat, SourceSetConfig, SourceSetPurpose, TestsConfig, ToolsConfig,
    };
    use crate::mcp::context::McpCallContext;
    use crate::mcp::port::DefaultMcpUseCasePort;
    use rmcp::transport::streamable_http_server::session::{local::LocalSessionManager, SessionId};
    use tokio_util::sync::CancellationToken;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn execute_tool_respects_configured_concurrency_limit() {
        let server = McpToolServer::with_port(
            Arc::new(test_config(1, 9)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let first = tokio::spawn(run_probe_call(
            server.clone(),
            McpTool::RunAllTests,
            active.clone(),
            max_active.clone(),
            CancellationToken::new(),
        ));
        let second = tokio::spawn(run_probe_call(
            server,
            McpTool::CheckSyntaxEdt,
            active,
            max_active.clone(),
            CancellationToken::new(),
        ));

        first.await.expect("first task join").expect("first call");
        second
            .await
            .expect("second task join")
            .expect("second call");

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_grace_period_uses_configured_value() {
        let config = test_config(3, 42);

        assert_eq!(max_concurrent_calls(&config), 3);
        assert_eq!(shutdown_grace_period(&config), Duration::from_secs(42));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_cancellation_returns_transport_error_without_running_call() {
        let server = McpToolServer::with_port(
            Arc::new(test_config(1, 9)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let execution_telemetry = server.telemetry.execution();
        let started = Arc::new(AtomicUsize::new(0));

        let first = tokio::spawn(run_probe_call(
            server.clone(),
            McpTool::RunAllTests,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            CancellationToken::new(),
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;

        let cancellation = CancellationToken::new();
        let started_clone = started.clone();
        let queued_cancellation = cancellation.clone();
        let second = tokio::spawn(async move {
            server
                .execute_tool(
                    McpTool::RunAllTests,
                    (),
                    queued_cancellation,
                    move |_, _, _, ()| {
                        started_clone.fetch_add(1, Ordering::SeqCst);
                        Ok(String::from("unexpected"))
                    },
                )
                .await
                .expect_err("queued call must be cancelled")
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        cancellation.cancel();

        let error = second.await.expect("second task join");
        first.await.expect("first task join").expect("first call");

        assert_eq!(
            error,
            execution_error(
                ErrorReason::Cancelled,
                ExecutionStage::Queued,
                Some(Duration::from_millis(300_000))
            )
        );
        assert_eq!(started.load(Ordering::SeqCst), 0);
        assert_eq!(execution_telemetry.snapshot().cancelled_total, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_timeout_returns_transport_error() {
        let server = McpToolServer::with_port(
            Arc::new(test_config_with_edt_timeout(1, 9, 20)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let execution_telemetry = server.telemetry.execution();
        let permit = server
            .concurrency_limit
            .clone()
            .acquire_owned()
            .await
            .expect("reserve the only execution slot");

        let started = Arc::new(AtomicUsize::new(0));
        let started_clone = started.clone();
        let error = server
            .execute_tool(
                McpTool::CheckSyntaxEdt,
                (),
                CancellationToken::new(),
                move |_, _, _, ()| {
                    started_clone.fetch_add(1, Ordering::SeqCst);
                    Ok(String::from("unexpected"))
                },
            )
            .await
            .expect_err("queued call must time out");
        drop(permit);

        assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        let data = error.data.as_ref().expect("queued timeout error data");
        assert_eq!(data["reason"], "timeout");
        assert_eq!(data["stage"], "queued");
        assert_eq!(data["timeoutMs"], 20);
        let wire = serde_json::to_value(&error).expect("serialize MCP error data");
        assert_eq!(wire["code"], rmcp::model::ErrorCode::INTERNAL_ERROR.0);
        assert_eq!(wire["data"]["reason"], "timeout");
        assert_eq!(wire["data"]["stage"], "queued");
        assert_eq!(wire["data"]["timeoutMs"], 20);
        assert_eq!(started.load(Ordering::SeqCst), 0);
        assert_eq!(execution_telemetry.snapshot().timeout_total, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_queued_cancellation_wins_before_deadline() {
        let server = McpToolServer::with_port(
            Arc::new(test_config_with_edt_timeout(1, 9, 80)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");

        let first = tokio::spawn(run_probe_call(
            server.clone(),
            McpTool::RunAllTests,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            CancellationToken::new(),
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;

        let started = Arc::new(AtomicUsize::new(0));
        let started_clone = started.clone();
        let cancellation = CancellationToken::new();
        let queued_cancellation = cancellation.clone();
        let second = tokio::spawn(async move {
            server
                .execute_tool(
                    McpTool::CheckSyntaxEdt,
                    (),
                    queued_cancellation,
                    move |_, _, _, ()| {
                        started_clone.fetch_add(1, Ordering::SeqCst);
                        Ok(String::from("unexpected"))
                    },
                )
                .await
                .expect_err("queued bounded call must be cancelled")
        });

        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation.cancel();

        let error = second.await.expect("second task join");
        first.await.expect("first task join").expect("first call");

        assert_eq!(
            error,
            execution_error(
                ErrorReason::Cancelled,
                ExecutionStage::Queued,
                Some(Duration::from_millis(80))
            )
        );
        assert_eq!(started.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn standard_tools_ignore_edt_timeout_budget_when_not_cancelled() {
        let server = McpToolServer::with_port(
            Arc::new(test_config_with_edt_timeout(1, 9, 20)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let execution_telemetry = server.telemetry.execution();

        let started = Instant::now();
        let result = server
            .execute_tool(
                McpTool::RunAllTests,
                (),
                CancellationToken::new(),
                move |_, _, _, ()| {
                    std::thread::sleep(Duration::from_millis(40));
                    Ok(String::from("ok"))
                },
            )
            .await;

        assert!(result.is_ok(), "standard tool should not time out");
        assert!(started.elapsed() >= Duration::from_millis(40));
        assert_eq!(execution_telemetry.snapshot().acquired_total, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn standard_running_cancellation_waits_for_terminal_state_and_releases_capacity() {
        let server = McpToolServer::with_port(
            Arc::new(test_config(1, 9)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let cancellation = CancellationToken::new();

        let first_server = server.clone();
        let first_active = active.clone();
        let first_max = max_active.clone();
        let first_cancel = cancellation.clone();
        let started = Instant::now();
        let first = tokio::spawn(async move {
            first_server
                .execute_tool(
                    McpTool::RunAllTests,
                    (),
                    first_cancel,
                    move |_, _, _, ()| {
                        let current = first_active.fetch_add(1, Ordering::SeqCst) + 1;
                        first_max.fetch_max(current, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(80));
                        first_active.fetch_sub(1, Ordering::SeqCst);
                        Ok(String::from("ok"))
                    },
                )
                .await
                .expect("running call should complete after cancellation request")
        });

        tokio::time::sleep(Duration::from_millis(15)).await;
        cancellation.cancel();
        let _result = first.await.expect("first task join");
        assert!(started.elapsed() >= Duration::from_millis(80));

        run_probe_call(
            server,
            McpTool::CheckSyntaxEdt,
            active,
            max_active.clone(),
            CancellationToken::new(),
        )
        .await
        .expect("second call");

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn running_cancellation_waits_for_terminal_state_and_releases_capacity() {
        let server = McpToolServer::with_port(
            Arc::new(test_config_with_edt_timeout(1, 9, 500)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let cancellation = CancellationToken::new();

        let first_server = server.clone();
        let first_active = active.clone();
        let first_max = max_active.clone();
        let first_cancel = cancellation.clone();
        let started = Instant::now();
        let first = tokio::spawn(async move {
            first_server
                .execute_tool(
                    McpTool::CheckSyntaxEdt,
                    (),
                    first_cancel,
                    move |_, _, _, ()| {
                        let current = first_active.fetch_add(1, Ordering::SeqCst) + 1;
                        first_max.fetch_max(current, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(80));
                        first_active.fetch_sub(1, Ordering::SeqCst);
                        Ok(String::from("ok"))
                    },
                )
                .await
                .expect("running call should complete after cancellation request")
        });

        tokio::time::sleep(Duration::from_millis(15)).await;
        cancellation.cancel();
        let _result = first.await.expect("first task join");
        assert!(started.elapsed() >= Duration::from_millis(80));

        run_probe_call(
            server,
            McpTool::RunAllTests,
            active,
            max_active.clone(),
            CancellationToken::new(),
        )
        .await
        .expect("second call");

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn running_timeout_waits_for_terminal_state_and_capacity_recovers() {
        let server = McpToolServer::with_port(
            Arc::new(test_config_with_edt_timeout(1, 9, 20)),
            Arc::new(DefaultMcpUseCasePort),
            McpCallContext::stdio(),
        )
        .expect("server");

        for _ in 0..2 {
            let started = Instant::now();
            let result = server
                .execute_tool(
                    McpTool::CheckSyntaxEdt,
                    (),
                    CancellationToken::new(),
                    move |_, _, _, ()| {
                        std::thread::sleep(Duration::from_millis(60));
                        Ok(String::from("ok"))
                    },
                )
                .await
                .expect("call should finish after timeout request reaches terminal state");
            assert_eq!(result.is_error, Some(false));
            assert!(started.elapsed() >= Duration::from_millis(60));
        }

        run_probe_call(
            server,
            McpTool::RunAllTests,
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            CancellationToken::new(),
        )
        .await
        .expect("capacity must recover after timed out calls");
    }

    #[tokio::test]
    async fn http_session_reservation_drop_releases_capacity() {
        let admission = HttpSessionAdmission::new(1, Arc::new(LocalSessionManager::default()));

        let reservation = admission
            .reserve_initialize()
            .await
            .expect("first reservation");
        drop(reservation);

        admission
            .reserve_initialize()
            .await
            .expect("capacity should recover after drop");
    }

    #[tokio::test]
    async fn http_session_reservation_prunes_stale_confirmed_sessions() {
        let admission = HttpSessionAdmission::new(1, Arc::new(LocalSessionManager::default()));

        let reservation = admission
            .reserve_initialize()
            .await
            .expect("first reservation");
        reservation.confirm(SessionId::from("stale-session"));

        admission
            .reserve_initialize()
            .await
            .expect("stale confirmed session should be pruned");
    }

    async fn run_probe_call(
        server: McpToolServer,
        tool: McpTool,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        cancellation: CancellationToken,
    ) -> Result<(), rmcp::ErrorData> {
        server
            .execute_tool(tool, (), cancellation, move |_, _, _, ()| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_active.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(String::from("ok"))
            })
            .await
            .map(|_| ())
    }

    fn known_hosts(allowed: &[&str]) -> KnownHosts {
        let mut config = test_config(1, 30);
        config.mcp.http.allowed_hosts = allowed.iter().map(|name| (*name).to_owned()).collect();
        KnownHosts::of(&config)
    }

    fn asking(target: &str, pairs: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().uri(target);
        for (name, value) in pairs {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).expect("a request")
    }

    fn headers(pairs: &[(&str, &str)]) -> Request<Body> {
        asking("/mcp", pairs)
    }

    #[test]
    fn the_loopback_is_answered_without_being_listed() {
        let known = known_hosts(&[]);
        for host in ["127.0.0.1:3000", "localhost:3000", "[::1]:3000"] {
            assert_eq!(
                admission_verdict(&headers(&[("host", host)]), &known),
                Ok(()),
                "{host} is answered"
            );
        }
    }

    #[test]
    fn a_page_that_rebound_its_own_name_is_refused() {
        // Та самая атака: браузер разрешает `evil.com` в `127.0.0.1` и стучится
        // к слушателю как к своему. Подделать `Host` он не может — на этом и ловим.
        let known = known_hosts(&[]);
        for host in [
            "evil.com:3000",
            "127.evil.com:3000",
            "127.0.0.1.nip.io:3000",
        ] {
            assert_eq!(
                admission_verdict(&headers(&[("host", host)]), &known),
                Err(RefusedHeader::Host),
                "{host} is refused"
            );
        }
    }

    #[test]
    fn the_name_in_an_absolute_request_target_is_checked_too() {
        // У http/1.1 имя хоста приезжает двумя каналами, и `Host` — не единственный:
        // absolute-form несёт своё, а hyper его в заголовок не переносит.
        let known = known_hosts(&[]);

        assert_eq!(
            admission_verdict(
                &asking("http://evil.com/mcp", &[("host", "127.0.0.1:3000")]),
                &known
            ),
            Err(RefusedHeader::Host)
        );
        assert_eq!(
            admission_verdict(
                &asking("http://127.0.0.1:3000/mcp", &[("host", "127.0.0.1:3000")]),
                &known
            ),
            Ok(())
        );
    }

    #[test]
    fn an_origin_shaped_unlike_a_browser_origin_is_refused() {
        // Браузер сериализует источник как `схема://хост[:порт]`. Ни userinfo, ни
        // путь оттуда прийти не могут, и петлевой хост за `@` — не тот источник.
        let known = known_hosts(&[]);
        let local = ("host", "127.0.0.1:3000");
        for origin in [
            "http://evil.com@127.0.0.1",
            "http://127.0.0.1/some/path",
            "http://127.0.0.1/?x=1",
        ] {
            assert_eq!(
                admission_verdict(&headers(&[local, ("origin", origin)]), &known),
                Err(RefusedHeader::Origin),
                "{origin} is refused"
            );
        }
    }

    #[test]
    fn a_host_that_is_missing_or_doubled_is_refused() {
        let known = known_hosts(&[]);
        assert_eq!(
            admission_verdict(&headers(&[]), &known),
            Err(RefusedHeader::Host)
        );
        // Повтор запрещён RFC 9112 §3.2, а `HeaderMap::get` молча берёт первый.
        assert_eq!(
            admission_verdict(
                &headers(&[("host", "127.0.0.1:3000"), ("host", "evil.com:3000")]),
                &known
            ),
            Err(RefusedHeader::Host)
        );
    }

    #[test]
    fn an_origin_is_checked_when_it_is_there_and_not_demanded_when_it_is_not() {
        let known = known_hosts(&[]);
        let local = ("host", "127.0.0.1:3000");

        assert_eq!(admission_verdict(&headers(&[local]), &known), Ok(()));
        // Любой петлевой порт: страница на другом порту петли — не та атака,
        // а межпортовый запрос браузер и так гасит на CORS.
        assert_eq!(
            admission_verdict(
                &headers(&[local, ("origin", "http://127.0.0.1:6274")]),
                &known
            ),
            Ok(())
        );
        for origin in ["http://evil.com", "null", "file://", "not an origin"] {
            assert_eq!(
                admission_verdict(&headers(&[local, ("origin", origin)]), &known),
                Err(RefusedHeader::Origin),
                "{origin} is refused"
            );
        }
    }

    #[test]
    fn a_listed_host_is_answered_however_its_root_dot_is_written() {
        let known = known_hosts(&["runner", "10.0.0.5"]);
        for host in [
            "runner:3000",
            "runner.:3000",
            "RUNNER:3000",
            "10.0.0.5:3000",
        ] {
            assert_eq!(
                admission_verdict(&headers(&[("host", host)]), &known),
                Ok(()),
                "{host} is answered"
            );
        }
        assert_eq!(
            admission_verdict(&headers(&[("host", "runner.evil.com:3000")]), &known),
            Err(RefusedHeader::Host)
        );
    }

    #[test]
    fn a_listed_host_also_names_an_origin() {
        let known = known_hosts(&["runner"]);
        assert_eq!(
            admission_verdict(
                &headers(&[("host", "runner:3000"), ("origin", "http://runner:3000")]),
                &known
            ),
            Ok(())
        );
    }

    fn test_config(max_concurrent_calls: usize, shutdown_grace_period_secs: u64) -> AppConfig {
        test_config_with_edt_timeout(max_concurrent_calls, shutdown_grace_period_secs, 300_000)
    }

    fn test_config_with_edt_timeout(
        max_concurrent_calls: usize,
        shutdown_grace_period_secs: u64,
        edt_timeout_ms: u64,
    ) -> AppConfig {
        AppConfig {
            base_path: PathBuf::from("/tmp/project"),
            work_path: PathBuf::from("/tmp/work"),
            execution_timeout: edt_timeout_ms,
            format: SourceFormat::Designer,
            providers: Default::default(),
            provider_origins: Default::default(),
            infobase: crate::config::model::InfobaseConfig::file("File=/tmp/ib"),
            source_sets: vec![SourceSetConfig {
                name: String::from("main"),
                purpose: SourceSetPurpose::Configuration,
                path: PathBuf::from("."),
            }],
            build: BuildConfig::default(),
            tools: ToolsConfig {
                platform: PlatformToolConfig::default(),
                enterprise: Default::default(),
                edt_cli: crate::config::model::EdtCliConfig {
                    command_timeout_ms: edt_timeout_ms,
                    ..Default::default()
                },
                ..Default::default()
            },
            mcp: McpConfig {
                http: McpHttpConfig::default(),
                execution: McpExecutionConfig {
                    max_concurrent_calls,
                    shutdown_grace_period_secs,
                },
            },
            tests: TestsConfig::default(),
        }
    }
}
