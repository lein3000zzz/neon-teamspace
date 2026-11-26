---
title: Observability
description: The server's logs, metrics, traces and profiles, its internal admin listener, and how it shuts down.
---

The server reports through one crate, `neon-observability`, which any crate but
`neon-domain` may use. It is built on the Rust ecosystem's own facades: `tracing` for logs
and spans, `metrics` for counters, gauges and histograms, and OpenTelemetry for exporting
traces. Code reports through the facades, and the crate decides once, at start, where it all
goes.

`neon serve` installs it first, before it reaches the database, and `neon migrate` does the
same. Its settings are under `observability` in [the configuration](/docs/self-hosting/configuration/#the-observability-section),
in five sections: `logging`, `metrics`, `tracing`, `profiling` and `admin`. An error in them,
such as a `sample_ratio` above 1, stops the server before it starts, together with the rest
of the configuration's errors. A setting that only has no effect, such as an `endpoint`
while export is off, is a warning, logged once logging is up.

## Logs

Logs go to standard output, one JSON object per line, or as indented text for reading in a
terminal during development, with `logging.format: pretty`. Each JSON line has `timestamp`,
`level`, `target`, `message` and the event's `fields`, and, inside a span, the span's fields
under `span`.

The level, `logging.level`, is a filter directive, such as `info` or `info,sqlx=warn`, which
sets a level for the server and quieter ones for chatty libraries. The default,
`info,sqlx::postgres::notice=warn,opentelemetry=warn`, is `info` with Postgres notices, such
as the one about the migrations table on every start, and OpenTelemetry's own messages kept
to warnings. A level that is set replaces the default whole.

The level filters what is written, not what is traced. Spans at `info` and above are always
traced, so a quiet level never costs trace IDs. A more verbose level, such as `debug` or
`info,neon_server=debug`, lowers the traced level with it, down to the most verbose level
the directive names, so lines logged inside `debug` spans carry IDs too.

Every line written inside a span carries the span's `trace_id` and `span_id` at the top
level, the same IDs the exported trace has. A log line found by searching leads to its
trace, and a slow trace leads to the lines it logged. With `debug` for the server's own
target, as in `info,neon_server=debug`, it logs one line for every request it answers, with
the status and how long it took. A handler that panics or runs past its time limit is
logged at `error`, inside the request's span.

Logs hold no secrets: passwords, tokens and the values of secret settings are never logged,
and request spans record the route, not the path or the query string; see
[Traces](#traces). Nor do they hold pages' content or values. The MCP SDK's `debug` lines
hold each request and answer whole, so it logs at `info` at most, unless the level names its
target, `rmcp`.

The commands that print their output, `neon openapi`, `neon settings docs` and
`neon config check`, install no logging, so standard output holds nothing but that output.
A configuration error found before logging is up, in any command, goes to standard error,
and the command exits with status 1.

## Metrics

Metrics are kept in a Prometheus registry and served at `/metrics` on the
[admin listener](#the-admin-listener). Every name starts with `neon_`. With
`metrics.enabled: false`, nothing is recorded and `/metrics` is not served.

### RED metrics

Each subsystem times its operations the same way, with three metrics for **r**ate,
**e**rrors and **d**uration:

| Metric | Type | Labels | What it counts |
|---|---|---|---|
| `neon_<subsystem>_operations_total` | counter | `operation`, `status` | Operations finished. |
| `neon_<subsystem>_operation_duration_seconds` | histogram | `operation`, `status` | How long they took. |
| `neon_<subsystem>_operations_in_flight` | gauge | `operation` | Operations started and not yet finished. |

`status` is `ok` or `error`. `operation` is a short, fixed name, never a value from a
request, so the number of label sets stays small. These subsystems record them:

| Subsystem | `operation` | Counted as `error` | Histogram buckets |
|---|---|---|---|
| `http`, the API | The method and the route template, such as `GET /api/v1/meta`. A request no route matches is `unmatched`, and an unknown method is `_OTHER`. | A 5xx answer, a timeout and a caught panic included, or a request dropped before its answer. A 4xx answer is `ok`. | 5 ms to 5 s |
| `jobs`, the job queue | The job's kind, such as `mail.send`. | A run that leaves the job dead, because it failed permanently or on its last attempt, a run that panicked, and a run cancelled before it ended. A failed run that will be retried is `ok`. | 100 ms to 2 min |
| `auth`, the account use cases | The use case, such as `login` or `create_invite`. | A failure of the server, such as a database that does not answer. A refusal, such as a wrong password, is `ok`. | 5 ms to 5 s |
| `workspaces`, the workspace use cases | The use case, such as `create_workspace` or `accept_invitation`. | A failure of the server. A refusal of the rules, such as `last_owner` or `member_limit`, is `ok`. | 5 ms to 5 s |
| `pages`, the page use cases | The use case, such as `create_page` or `share_page`. | A failure of the server. A refusal of the rules is `ok`. | 5 ms to 5 s |
| `agents`, the agent use cases | The use case, such as `create_agent`, `transfer_agent` or `create_agent_token`. | A failure of the server. A refusal of the rules, such as `agent_limit`, is `ok`. | 5 ms to 5 s |
| `oauth`, the OAuth server | The use case, such as `register`, `authorize`, `approve`, `token`, `revoke` or `cleanup`. | A failure of the server. A refused request, a wrong verifier or a reused code is `ok`. | 5 ms to 5 s |
| `databases`, the database use cases | The use case, such as `create_row`, `query`, `update_property` or `follow`, the check of a live feed's access. | A failure of the server. A refusal of the rules, an invalid value or filter, or a type change that would lose values is `ok`. | 5 ms to 5 s |
| `sync`, the live documents | `load`, `persist`, a batch of updates logged, and `compact`, a snapshot saved. | A database that fails, or a saved update that does not decode. | 5 ms to 5 s |
| `transfer`, imports and exports | `admit_import`, `import`, `export_page` and `export_teamspace`. | A failure of the server, such as a document that does not read as blocks. A refusal, a limit or the time limit is `ok`. | 100 ms to 5 min |
| `mcp`, the MCP server | The tool's name, such as `read_page` or `claim_task`, and `list_resources`, `read_resource` and `subscribe`. | A failure of the server. A tool's refusal, such as `not_found` or `already_claimed`, or arguments its schema does not take, is `ok`. | 5 ms to 5 s |
| `webhooks`, webhooks' use cases, deliveries and requests | The use case, such as `create_webhook`, `test_webhook` or `create_incoming`; `deliver`, an attempt at a delivery; `receive`, a request to an incoming webhook. | A failure of the server. A refusal, a receiver that fails or times out, and a request refused for its signature, payload or rate are `ok`. | 5 ms to 10 s |

Deliveries are jobs of the kind `webhooks.deliver`, so the `jobs` subsystem counts their runs
too: a run that the receiver fails is retried and `ok`, the last failed one `error`.

The `sync` subsystem's `read` and `edit` operations time the server's own reads and edits of
pages' content, for the markdown API and the MCP tools.

Timing an operation starts a guard. Finishing the guard records the outcome; a guard dropped
without being finished, because the code returned early or the work was cancelled, counts as
an error. A path that forgets to report its result therefore shows up as errors, not as
nothing.

For example, the share of each route's requests that failed over five minutes:

```txt
sum by (operation) (rate(neon_http_operations_total{status="error"}[5m]))
  / sum by (operation) (rate(neon_http_operations_total[5m]))
```

### Gauges

Beside the RED metrics, gauges show the state of what the server holds. While metrics are
on, a background task samples them as the server starts and every 15 seconds after, and
stops before the database pool closes:

| Metric | Labels | What it shows |
|---|---|---|
| `neon_db_pool_connections` | `state`: `idle` or `in_use` | The connections the pool holds, waiting or lent to a query or transaction. |
| `neon_db_pool_max_connections` | | The most the pool opens: `database.max_connections` minus the one the job worker keeps for listening. |
| `neon_jobs_queue_depth` | `status`: `queued`, `running`, `done` or `dead` | Jobs in the queue table, from one count grouped by status. Done and dead jobs count until they are deleted, seven days after they finished. |

The live documents keep their own gauges, set as they change rather than sampled:
`neon_sync_live_documents`, the page documents held in memory, and
`neon_sync_open_connections`, the open WebSocket connections. [Real-time
sync](/docs/architecture/realtime/#metrics) lists them with the histogram of message sizes
and the counters of messages and disconnects. The database feeds keep theirs likewise:
`neon_databases_open_connections`, and the counters of [database
feeds](/docs/architecture/realtime/#database-feeds). No log line or span holds a property's
value: they carry ids and counts. The MCP server keeps `neon_mcp_sessions`, its sessions
open, and `neon_mcp_subscriptions`, the resources they subscribe to.

## Traces

Spans are made with `tracing` and exported over OTLP, the OpenTelemetry protocol, through
`tracing-opentelemetry`, to a collector such as Grafana's or Jaeger's. Export is off unless
`tracing.enabled` is `true`. It then sends spans over OTLP/HTTP with protobuf bodies to
`tracing.endpoint`, or, when that is unset, where the standard `OTEL_EXPORTER_OTLP_*`
variables say, else `http://localhost:4318`. `tracing.sample_ratio` sets the share of new
traces exported. Spans and their IDs exist with export on or off, so logs stay correlated
and trace context still passes on.

Each request has a server span named after its method and route template, such as
`GET /api/v1/pages/{id}`. The span's `url.path` is the route template too, never the path
the client sent, and the query string is not recorded at all: paths and queries carry
secrets, such as webhook tokens and invite codes, and every log line inside the request
repeats the span's fields. The span also records the status, the user agent and the request
ID, which is the client's `x-request-id` when it is short printable text, or a new UUID, and
which the response carries back.

Trace context crosses process and time boundaries in the W3C formats, TraceContext
(`traceparent`) and Baggage:

```mermaid
flowchart LR
  client[Client] -- "traceparent, linked or continued" --> http[HTTP request span]
  http -- "traceparent, in the job row" --> job[Job run span]
  job -- "traceparent header" --> hook[Outgoing webhook]
```

- **Incoming HTTP.** The API's port is public, so by default a request starts a new trace,
  sampled by `sample_ratio`. A `traceparent` the client sent is recorded as a link on the
  request's span, and its `baggage` is dropped. Trusting it instead would let any client
  force sampling past `sample_ratio`, attach spans to someone else's trace, and pass forged
  baggage on to job rows and webhooks. With `tracing.trust_incoming: true` a request
  continues the caller's trace and keeps its baggage; turn it on only when every caller is
  trusted, for example behind a proxy that removes or rewrites these headers.
- **Jobs.** Queueing a job stores the current span's `traceparent` in its row, unless the
  caller passes one, so a job queued while handling a request keeps the request's trace.
  The worker runs it in a span named `job <kind>`, such as `job mail.send`, whose parent
  comes from the row, so the run continues the request's trace minutes or hours later. A job
  queued outside any span has no `traceparent`, and its run starts a new trace.
- **Outgoing webhooks.** A delivery is a job, queued in the transaction of the change that
  caused it, so its run, in a span named `job webhooks.deliver` with a child
  `webhooks deliver`, continues that change's trace. Its request carries the run's
  `traceparent`, so a receiver that traces joins the trace too. A request to an incoming
  webhook, in a span named `webhooks receive`, starts a trace of its own as any request does.

### Expected and unexpected errors

Errors come in two kinds, and traces keep them apart:

- **Expected errors** are answers: not found, forbidden, invalid input, a conflict, a job
  that failed and will be retried. They are part of normal operation. The span records them
  in the attributes `neon.error.expected` and `neon.error.reason`, and its status stays
  unset. A 4xx answer is an expected error.
- **Unexpected errors** are faults: the database is unreachable, a query fails, a bug. They
  set the span's status to error. A 5xx answer, a request dropped before its answer, a job
  that failed for good or panicked, and any line logged at `error` are unexpected errors.
  A line at `warn`, even with an `error` field, leaves the status alone.

A trace marked as failed then means the server went wrong, not that someone asked for a page
that does not exist.

## Profiles

The server can profile itself, in one of three modes, set in `profiling.mode`:

| Mode | What it does |
|---|---|
| `off` | No profiling. The default. |
| `pull` | Serves profiles in pprof's format on the admin listener, for a scraper or `pprof` itself to fetch. |
| `push` | Sends profiles from inside the process to the Pyroscope server at `profiling.push.server_address`, every `upload_interval_seconds`, labelled with `profiling.push.tags`. |

`profiling.profiles` lists what to collect, `cpu`, `heap` or both. Only one mode runs at a
time, because two CPU profilers in one process break each other's sampling.

In `pull` mode, `/debug/pprof/profile?seconds=N` records a CPU profile over `N` seconds, 30
unless set and 60 at most. One runs at a time; a second request gets `409`. Shutting down
cuts a running profile short. `/debug/pprof/heap` answers the sampled live heap, gzipped.
Both open with `go tool pprof http://127.0.0.1:9090/debug/pprof/profile` or any pprof viewer.

### Heap profiles

Heap profiles come from jemalloc's sampling profiler and only work on Linux. Three things
are needed:

1. **A Linux build.** The `neon` binary uses jemalloc as its allocator on Linux, through
   `neon-observability`'s wrapper, which tells the profiler jemalloc serves the
   allocations. Every Linux build compiles jemalloc from its C sources, so the build stage
   needs a C compiler and `make`. The `rust:*-slim` images have gcc but not `make`.
2. **Profiling switched on in jemalloc when the process starts**, with the variable
   `_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_sample:19`. jemalloc reads it once.
   `lg_prof_sample:19` samples one allocation in every 512 KiB on average, which keeps the
   overhead small.
3. **`heap` in `profiling.profiles`**, with `pull` or `push` as the mode.

Without one of these, `/debug/pprof/heap` answers `501` with the reason, and push mode skips
the heap profile with a warning. On other systems, `heap` in `profiling.profiles` is also a
warning at start.

## The admin listener

The admin listener is a second HTTP server inside the process, on its own address,
`127.0.0.1:9090` unless `admin.addr` says otherwise (`NEON_OBSERVABILITY__ADMIN__ADDR`). It
starts before the server reaches its database, so `/livez` and `/metrics` answer while
migrations run. It is internal: Caddy never exposes it, and it should be bound to loopback or
a private network only, as profiles reveal the server's memory and a CPU profile holds the
process for its duration. The server logs a warning when it is bound anywhere else.
`admin.enabled: false` turns it off.

| Path | What it answers |
|---|---|
| `/metrics` | The metrics, in Prometheus's text format. |
| `/livez` | `200` while the process runs. A failing check means it should be restarted. |
| `/readyz` | `200` while the server can take requests; `503` with the reason otherwise. A failing check means traffic should go elsewhere for now. |
| `/debug/pprof/*` | Profiles, in `pull` mode. |

`/readyz` answers `503 starting` until the HTTP server listens. From then on it pings
Postgres on each check and answers `503` when the database does not answer within two
seconds or cannot be reached. From the moment shutdown begins it answers
`503 shutting down`, whatever the database says.

Keeping these apart from the app's port means the public origin has no metrics, probes or
profiles to protect, and a scraper or an orchestrator reaches them without credentials.

## Shutdown

Each part of the server registers how it stops as it starts, with `neon-observability`'s
shutdown registry, and the registry stops them in the reverse order, the last started
first. On SIGTERM, or Ctrl-C in a terminal, it runs:

1. `/readyz` turns to `503`, so a load balancer that checks it stops sending traffic. The
   server does not wait for it to notice: the next step follows at once, and connections
   that arrive from then on are refused.
2. The HTTP server stops accepting connections and lets the requests in flight finish, for
   up to 30 seconds, then closes any still open. Open WebSocket connections are not
   requests in flight; they stay until step 5.
3. The job worker claims no more jobs and gives those it is running up to 30 seconds. A job
   still running then is cancelled and queued again, and the attempt does not count.
4. The gauges stop sampling, when metrics are on.
5. The live documents close their WebSocket connections with `1012`, so clients reconnect
   to the next server, and save their snapshots, for up to 30 seconds; see
   [Real-time sync](/docs/architecture/realtime/#stopping).
6. The database pool closes, as everything before it uses it until it stops.
7. The admin listener stops, so `/metrics` stays up while the rest drains.
8. The telemetry flushes last: the profiler stops, the trace exporter sends the spans it
   still holds, and the log writer writes the lines it has buffered. What the other parts
   logged while stopping, and the spans of the last requests and jobs, still go out.

Every step runs even when one before it fails. Each failure is logged as it happens, and
the process then exits with status 1, naming every step that failed. Each step has a time
limit, 10 seconds, or 10 seconds beyond its own 30 for the HTTP server, the job worker and
the live documents,
and beyond the upload interval for the profiler in `push` mode, whose agents send their last
batches side by side at their next upload. One that hangs is abandoned and the next one
starts. Shutting down twice does nothing the second time. A server that fails while
starting runs the same steps for the parts already started.

## The stack

The Compose file's `observability` profile runs Grafana's all-in-one OpenTelemetry image,
`grafana/otel-lgtm`, with Prometheus, Tempo, Loki and Pyroscope, beside the server or beside
`make run`, and provisions a Grafana dashboard of the RED metrics and the gauges;
[Observability stack](/docs/self-hosting/observability/) covers running and reading it.
