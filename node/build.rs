// napi-build emits the linker flags the Node-API runtime needs (so the
// `.node` artifact resolves Node's exported symbols at load time
// instead of pulling them in at link time).
extern crate napi_build;

fn main() {
    napi_build::setup();
}
