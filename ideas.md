Best Borrowed Ideas

- Podman: daemonless operation, rootless-by-default, pods, remote API/client, and systemd-native deployment via
`  Quadlet. Sources: Podman overview (https://docs.podman.io/en/v5.1.1/), rootless`
  (https://docs.podman.io/en/v5.2.0/markdown/podman-run.1.html), Quadlet
  (https://docs.podman.io/en/latest/markdown/podman-systemd.unit.5.html)
- nerdctl/containerd: lazy image pulling, image signing/verification, encrypted images, rootfs mode, and multi-n
  etwork support. These are strong differentiators over plain Docker UX. Source: nerdctl features
  (https://github.com/containerd/nerdctl)
- Incus: one tool for both containers and VMs, image-server workflow, snapshots, profiles, projects, clustering,
  and richer storage/networking. This is especially relevant if Corten wants both containers and machine images.
  Sources: Incus overview (https://linuxcontainers.org/incus/docs/main/), images
  (https://linuxcontainers.org/incus/docs/main/image-handling/), profiles
  (https://linuxcontainers.org/incus/docs/main/profiles/), projects
  (https://linuxcontainers.org/incus/docs/main/projects/), clustering
  (https://linuxcontainers.org/incus/docs/main/explanation/clustering/)
- gVisor: sandboxed runtime class, checkpoint/restore, runtime monitoring, userspace network/filesystem isolatio
  n. Great model for a “secure mode.” Sources: gVisor overview (https://gvisor.dev/docs/), runtime monitoring
  (https://gvisor.dev/docs/user_guide/runtimemonitor/), filesystem
  (https://gvisor.dev/docs/user_guide/filesystem/)
- Kata Containers: lightweight-VM isolation for sensitive workloads, while keeping OCI/container workflows. Very
  relevant if Corten grows a “machine-grade isolation” mode. Source: Kata Containers (https://katacontainers.io/)

What Corten Should Implement

- Rootless-first runtime and storage model
- Daemonless local operation with an optional API server
- Pods or grouped workloads as a first-class concept
- Systemd-native units, not just run commands
- Fast image startup via lazy pulling/snapshotters
- Signed and verified images by default
- Per-container writable snapshots, clones, and fast rollback
- Profiles/projects for reusable policy and multi-tenant separation
- Unified containers + machines if you pursue that direction
- Multiple isolation modes: native, sandboxed, microVM

My Priority Order

1. Podman-style rootless, daemonless, systemd-friendly runtime
2. nerdctl-style lazy pull, signing, encryption, rootfs support
3. Incus-style snapshots, profiles, projects, image repositories
4. gVisor/Kata-style hardened runtime classes
5. Docker-style enterprise controls: RBAC, audit logs, registry policy

If I had to define Corten in one sentence, I’d aim for:
“Podman UX, nerdctl image tech, Incus image/machine model, and optional gVisor/Kata isolation.”

If you want, I can turn this into a concrete Corten feature matrix comparing Docker, Podman, Incus, nerdctl,
gVisor, and Kata side by side.

