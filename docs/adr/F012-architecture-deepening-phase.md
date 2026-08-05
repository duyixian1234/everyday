# ADR: Architecture Deepening Phase (v0.11–v0.12)

**Status**: Accepted (Phase 1 implemented 2026-08-05; Phases 2–3 pending)

**Date**: 2026-08-05

**Deciders**: Everyday Architecture Review (codebase-design skill)

**Participants**: Development team, module owners

---

## Implementation Status

| Item | Phase | Status | Notes |
|------|-------|--------|-------|
| P6 TypedValue (`Output::TypedRecords`) | 1 | **Done** (commit `4264909`) | `TypedValue` Text/Number/Boolean/Null; JSON keeps native types; `mail list` uid/unread typed, `memory list`/`history` confidence typed; backward compat via existing `Records` variant |
| P2c Config validation | 1 | **Done** (commit `d3de1be`) | `Config::validate()` inside `load_from()`: default-account references, required fields, provider whitelist, Notion ID shape |
| P2a AccountProvider trait | 1 | **Done** (commit `274143e`) | `AccountProvider` single resolution algorithm on the five account configs; `Config::resolve_account_name()`; ops_log delegates; `X_account()` kept for backward compat; `impl_account_lookup!` macro (R007) removed |
| P1 CLI/business separation | 2 | Pending | v0.11.0 workstream |
| P2b Config subsets | 2 | Pending | v0.11.0 workstream |
| P3 Lifecycle hooks | 3 | Pending | v0.12.0 |
| P4 Request context | 3 | Pending | v0.12.0 (breaking) |
| P5 Middleware stack | 3 | Pending | v0.12.0 |

---

## Summary

Execute a multi-phase architecture deepening initiative to improve module interface design, reduce complexity, enhance testability, and enable new capabilities (REPL, API layer). Guided by deep-module design principles: small interface, large hidden behavior.

**Scope**: 12 improvements across core abstractions (Executor, ModuleRegistry, Output, Config) and all 11 modules (mail, cal, rss, note, todo, bookmark, timeline, search, auth, config, memory).

**Timeline**: 4–6 weeks (v0.11.0, v0.11.1, v0.12.0)

**Breaking Changes**: Minimal in v0.11; significant in v0.12 (but gated to major version bump).

---

## Context

### Current State (v0.10.x)

**Architecture**:
- 11 modules implement `Executor` trait (20–50 lines of boilerplate ArgSpec per module)
- Argument parsing duplicated: defined in ArgSpec, then parsed again in execute()
- All modules receive full `Arc<Config>` (tight coupling)
- Account resolution logic spread across 5 macro-generated methods
- No config validation at load time (errors caught at runtime)
- No module lifecycle management (initialize, health_check, shutdown)

**Specific Issues**:
1. **ArgSpec Boilerplate**: mail.rs alone has 85 lines of `ActionArgSpec` definitions
2. **Tight Config Coupling**: All 8 modules import Arc<Config>, can't test with partial config
3. **Account Lookup Duplication**: Each module re-implements `--account override → default → error`
4. **Output Type Loss**: All tabular values converted to strings; JSON loses numeric types
5. **No Lifecycle**: Modules can't initialize resources or report health

**Measured Depth**:
- Executor: Deep (small interface, large behavior) ✓
- ModuleRegistry: Shallow (just a HashMap wrapper) ✗
- Config: Shallow (scattered account resolution, no validation) ✗
- Output: Deep (unified rendering) ✓
- Search: Deep (orchestrates 7 providers) ✓

### Why This Matters

**Current Pain Points**:
- Adding a new action requires ~50 lines of boilerplate (ArgSpec) per module
- Testing module behavior requires invoking through CLI + parsing Output (slow, brittle)
- Can't build REPL, API layer, or batch tools without reimplementing dispatch logic
- Config errors hidden until runtime (e.g., missing default account)
- Can't monitor module health or gracefully restart

**Business Impact**:
- Slow feature velocity (boilerplate overhead)
- High defect risk (parsing duplicate + testing through CLI)
- Limited extensibility (new frontends require duplicate dispatch logic)
- Poor observability (no health checks, no request tracing)

---

## Decision

Execute a three-phase architecture deepening initiative to deepen module interfaces and reduce complexity:

### Phase 1: Quick Wins (v0.11.0-rc, 2 days)
- **P2c**: Add config validation at load time
  - Catch semantic errors immediately (e.g., missing default account)
  - Validate Notion page IDs, provider strings, etc.

- **P2a**: Extract account lookup trait
  - Move from 5 macro-generated methods to unified AccountProvider trait
  - Single source of truth for account resolution logic

- **P6**: Preserve types in Records (TypedValue)
  - Add TypedValue enum (Text, Number, Boolean, Null)
  - JSON output preserves numeric types instead of converting to strings

**Rationale**: Low-risk improvements with immediate value (validation, cleaner code, better JSON).

### Phase 2: Core Refactoring (v0.11.0, 3 weeks)
- **P1**: Separate CLI from business logic
  - Extract `ModuleService` trait: domain methods return domain types (no Output)
  - Extract `ModuleCliInterface` trait: minimal CLI binding (single CliAction const per action)
  - Eliminate ArgSpec boilerplate (~70% reduction)
  - Enable testing service methods directly without CLI parsing

  *Rationale*: Deepens Executor by splitting two concerns; each trait is now smaller and more focused. Leverage increases: modules can be tested directly; REPL/API layers become possible.

- **P2b**: Inject config subsets, not full Config
  - Create MailModuleConfig, CalendarModuleConfig, etc.
  - Modules depend only on their own section
  - Reduces coupling; enables testing with partial config

  *Rationale*: Seam placement: Config should be injected at module initialization, not queried globally. Reduces hidden dependencies.

### Phase 3: Advanced Features (v0.11.1–v0.12.0, 1 week)
- **P3**: Module lifecycle hooks
  - Add initialize(), health_check(), shutdown() to Executor
  - Enable resource management, health monitoring, graceful shutdown

- **P4**: Request context propagation
  - Pass RequestContext (request_id, deadline, caller) through execute()
  - Enable tracing, deadline enforcement, permissions checking

- **P5**: Middleware stack
  - Layer logging, metrics, retry logic between main and modules
  - Centralize cross-cutting concerns; eliminate duplication across modules

  *Rationale*: Deepen Executor further by moving cross-cutting concerns to a separate layer. Middleware is deep: small interface (before, after, on_error), large hidden behavior (logging, retry, timing).

---

## Rationale

### Deep Module Design Principles Applied

1. **Interface Simplicity**: 
   - Executor shrinks from 3 methods to 1 business method (per action)
   - ModuleService has domain method, not string-based dispatch
   - SearchRegistry has 2 methods (register, query), not 10

2. **Leverage (Behavior per Unit Interface)**:
   - Before P1: 85 lines of ArgSpec for mail.rs; still need manual parsing in execute()
   - After P1: 25 lines of CliAction consts; dispatch is automatic
   - Caller doesn't need to know about clap, ArgKind, Positional

3. **Locality (Change Concentration)**:
   - Before: Config changes → update 8 module constructors
   - After P2b: Config changes → update one MailModuleConfig struct
   - Before: Account resolution logic → 5 duplicated methods
   - After P2a: Account resolution → single AccountProvider trait

4. **Seam Placement**:
   - Before: CLI concerns (ArgSpec) leak into business logic (Executor)
   - After P1: Clean seam between CLI (ModuleCliInterface) and business (ModuleService)
   - Before: Config passed globally; modules can access anything
   - After P2b: Config passed at seam; modules see only their section

### Risk Assessment

| Improvement | Risk | Mitigation |
|-------------|------|-----------|
| P1 (CLI/Business) | Medium (logic refactored 11 times) | Refactor one module at a time; thorough testing |
| P2a (Account trait) | Low (internal only) | Macro → trait substitution; backward compat wrapper |
| P2b (Config subsets) | Medium (constructor signatures change) | Update ModuleRegistry carefully; test with various configs |
| P2c (Validation) | Low (additive) | Validation only; no behavior change |
| P3 (Lifecycle) | Low (additive) | Default implementations; optional per module |
| P4 (Context) | Medium (API change) | Non-breaking in v0.11; breaking in v0.12 as separate major version |
| P5 (Middleware) | Medium (new abstraction) | Layer on top; modules unchanged if middleware disabled |
| P6 (TypedValue) | Low (non-breaking) | Backward compat; Records variant still available |

### Alternatives Considered

#### Alt 1: Do Nothing
- **Rejected**: Boilerplate and coupling pain will compound; new features (REPL, API) blocked

#### Alt 2: Quick Refactor (Only P1)
- **Rejected**: Leaves Config coupling, account duplication, and missing validation
- **Better to**: Combine with P2a, P2b, P2c for comprehensive improvement

#### Alt 3: Major Rewrite (All at Once)
- **Rejected**: High risk; long release cycle; no incremental feedback
- **Better to**: Staged phases (Option C) with releases between phases

#### Alt 4: Separate Layer Above Executor (Don't Change Executor)
- **Rejected**: Executor is the problem (leaks CLI concepts); better to fix root cause
- **Better to**: Refactor Executor as part of solution

---

## Consequences

### Positive

1. **Velocity**: ArgSpec boilerplate reduced ~70% → faster feature development
2. **Testability**: Service methods callable directly → simpler, faster tests
3. **Extensibility**: REPL, API layer, batch tools can reuse service layer without CLI plumbing
4. **Observability**: Health checks, request tracing, middleware support
5. **Maintainability**: Account lookup centralized; config validation explicit; no hidden coupling
6. **Code Quality**: Smaller, focused interfaces; better separation of concerns

### Negative (Tradeoffs)

1. **Effort**: 4–6 weeks of focused development
2. **Transition**: Users of module constructors need to update (only affects internal code)
3. **Testing**: Must add tests for new service layer (but offset by easier testing)
4. **Documentation**: Need migration guide for v0.12 breaking changes

### Mitigation

- **Effort**: Staged phases allow early releases; incremental feedback
- **Transition**: Careful migration; non-breaking in v0.11, breaking change gated to v0.12
- **Testing**: Community can test v0.11.0-rc before final release
- **Documentation**: ADR + migration guide included in each release

---

## Implementation Plan

### Phase 1 (v0.11.0-rc, 2 days)
1. Implement P2c (Config validation)
   - Add Config::validate() method
   - Call from load_or_default()
   - Add tests for common validation failures

2. Implement P2a (Account trait)
   - Define AccountProvider trait
   - Implement for MailConfig, CalendarConfig, NoteConfig, TodoConfig, BookmarkConfig
   - Create Config::resolve_account_name()
   - Keep old methods for backward compat

3. Implement P6 (TypedValue)
   - Add TypedValue enum
   - Update Output to support TypedRecords
   - Update render() to preserve types in JSON

**Deliverable**: v0.11.0-rc tag; ready for team review

### Phase 2 (v0.11.0 final, 3 weeks)
1. **Week 1**: P1 infrastructure + mail module
   - Define ModuleService, ModuleCliInterface, CliAction
   - Implement MailService trait
   - Extract MailListOptions, MailEnvelope, etc.
   - Rewrite MailModule::dispatch_cli()
   - Add tests calling mail_list() directly

2. **Week 2**: P1 remaining modules (cal, rss, note, todo, bookmark, timeline, search, auth, config, memory)
   - 1 day per simple module (cal, rss)
   - 1.5 days per complex module (note, todo, bookmark)
   - 0.5 days per special module (timeline, search, auth, config, memory)

3. **Week 3**: P2b + Cleanup
   - Create MailModuleConfig, etc.
   - Update ModuleRegistry::build()
   - Remove old code; finalize

**Deliverable**: v0.11.0 release; all boilerplate eliminated, all modules testable directly

### Phase 3 (v0.11.1–v0.12.0, 1 week)
1. Implement P3 (Lifecycle hooks)
2. Implement P4 (Request context)
3. Implement P5 (Middleware stack)

**Deliverable**: v0.12.0 release; full observability and middleware support

---

## Metrics for Success

### Code Reduction
- [ ] ArgSpec boilerplate reduced by 70% (mail: 85 lines → 25 lines)
- [ ] Account resolution duplicates → 1 (from 5 macros)
- [ ] Total lines of module impl code reduced by ~300 lines

### Testing
- [ ] Test coverage increases by 20% (new service method tests)
- [ ] All modules have direct service tests (not just CLI tests)
- [ ] Test execution time for module changes decreases by 50% (no CLI overhead)

### Observability
- [ ] All modules implement health_check()
- [ ] Request IDs flow through entire stack
- [ ] Middleware stack enabled with logging by default

### User Feedback
- [ ] No breaking changes in v0.11 (backward compat maintained)
- [ ] Clear migration guide for v0.12 (if users implement custom modules)

---

## Review Checklist

- [ ] Team agrees with problem statement (boilerplate, coupling, testability)
- [ ] Proposed solutions align with deep-module principles
- [ ] Risk mitigations are acceptable
- [ ] Timeline is realistic given team capacity
- [ ] Staging approach (Option C) is preferred
- [ ] No blockers identified

---

## Related Decisions

**Supersedes** (indirectly):
- F001: CLI shape (output abstraction) — deepens by separating business from CLI
- F003: Module scope — improves by reducing coupling
- R012: Config executor trait — improves by config subsets
- R013–R015: Auth module — maintains; works with new config structure
- R007: `impl_account_lookup!` macro — superseded by the `AccountProvider` trait (P2a)

**Reinforces**:
- S001–S006: Search architecture (good deep-module pattern to follow)
- L001–L013: Timeline orchestration (pattern for lifecycle + events)
- M001–M005: Mail cache (can be simplified with lifecycle hooks)

**Future Work (v1.0+)**:
- Config schema versioning (for future migrations)
- Search caching layer (when latency becomes issue)
- Plugin system (dynamic module loading)

---

## FAQ

### Q: Will this break existing user configs?
**A**: No. v0.11 maintains backward compat. v0.12 request context is internal-only; configs unaffected.

### Q: Must users update their custom modules?
**A**: Only if they implement Executor. Migration guide provided. Old Executor can work for 1–2 releases via compatibility layer.

### Q: Why split into 3 phases instead of doing all at once?
**A**: Risk reduction, incremental feedback, early value delivery. P1 alone (v0.11.0) gives 70% boilerplate reduction.

### Q: What if we discover major issues during Phase 1?
**A**: Can roll back P1 and ship v0.11.0 with just Phase 1 (P2c, P2a, P6). These are low-risk.

### Q: How does this affect performance?
**A**: No negative impact. P1–P6 are refactorings only. P3–P5 add layers but are optional/configurable. Middleware has minimal overhead (default: logging only).

### Q: Can modules skip health_check() or other lifecycle hooks?
**A**: Yes. Executor provides default implementations. Modules opt-in via override.

---

## Approval Record

| Role | Name | Date | Status |
|------|------|------|--------|
| Architecture Lead | TBD | — | Pending |
| Module Owner (Mail) | TBD | — | Pending |
| Module Owner (Config) | TBD | — | Pending |
| Development Lead | TBD | — | Pending |

---

## References

- **Analysis Documents**:
  - `CORE_ABSTRACTIONS_ANALYSIS.md` (deep-module evaluation)
  - `P1_REFACTORING_PROPOSAL.md` (CLI/business separation details)
  - `OUTPUT_CONFIG_SEARCH_ANALYSIS.md` (layer-by-layer analysis)
  - `IMPLEMENTATION_ROADMAP.md` (phase-by-phase execution plan)

- **External References**:
  - Ousterhout, John K. *A Philosophy of Software Design*. Chapter 10: Deep Modules
  - Feathers, Michael C. *Working Effectively with Legacy Code*. Seams and test points
  - Martin, Robert C. *Clean Architecture*. Dependency Inversion, Interface Segregation

- **Related ADRs** (in this repo):
  - [F001](F001-cli-shape.md) — CLI output abstraction
  - [F003](F003-module-scope-external-integration.md) — Module scope
  - [R012](R012-config-executor-trait.md) — Config as executor
  - [R013–R015](R013-auth-module-consolidation.md) — Auth consolidation
  - [S001–S006](S001-search-architecture.md) — Search architecture (good model)

