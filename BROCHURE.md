# Estate automation without command injection

In one line: reviewed estate declarations compiled into bounded, reproducible operations without executable-text injection.

`sim-estate` turns reviewed, typed exposure declarations into bounded operations. Callers choose an exposure id and shaped parameters; they never supply executable text or native resource identities. A deterministic model makes every lifecycle race and provider fault reproducible without a host.
