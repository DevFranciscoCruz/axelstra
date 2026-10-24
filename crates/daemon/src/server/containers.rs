use super::*;

pub(super) fn call(c: &mut Ctx) -> Result<Value> {
    let req = c.req;
    let (p, reg) = (c.p, c.reg);
    match req.method.as_str() {
        "docker.open_logs" | "docker.open_shell" => {
            let (s, label, meta) = crate::docker::terminal(&reg.store, &req.method, p)?;
            Ok(json!({"id": reg.spawn(&s, Some(&label), Some(meta))?}))
        }
        "docker.status" | "docker.containers" | "docker.stats" | "docker.images" | "docker.volumes" | "docker.networks"
        | "docker.inspect" | "docker.logs" | "docker.fs_list" | "docker.fs_read" | "docker.action" => {
            tokio::task::block_in_place(|| crate::docker::call(&reg.store, &req.method, p))
        }
        m => anyhow::bail!("unknown method {m}"),
    }
}
