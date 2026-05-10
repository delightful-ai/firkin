## firkin-e2b-server

`firkin-e2b-server` owns local compatibility servers and in-memory control-plane
machinery: Hyper HTTP dispatch, domain proxy, envd-compatible process/filesystem
servers, registries, lifecycle scheduler, and local state envelopes.

It may depend on `firkin-e2b-wire` and runtime-facing contracts. It must not
become an Apple/VZ runtime adapter or product SaaS API server.

Do not put VM mechanics, runtime scheduling, benchmark policy, or public
product route/auth/tenant semantics here.
