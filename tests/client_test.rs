extern crate etcd;
extern crate futures;
extern crate hyper;
extern crate native_tls;
extern crate tokio_core;

use std::fs::File;
use std::io::Read;
use std::ops::Deref;
use std::thread::{sleep, spawn};
use std::time::Duration;

use futures::Future;
use hyper::client::{Connect, HttpConnector};
use native_tls::{Certificate, Pkcs12, TlsConnector};
use tokio_core::reactor::{Core, Handle};

use etcd::{Client, Error};
use etcd::kv::{self, KeySpaceInfo};

/// Wrapper around Client that automatically cleans up etcd after each test.
struct TestClient<C> where C: Clone + Connect {
    c: Client<C>,
}

impl TestClient<HttpConnector> {
    /// Creates a new client for a test.
    fn new(handle: &Handle) -> TestClient<HttpConnector> {
        TestClient {
            c: Client::new(handle, &["http://etcd:2379"], None).unwrap(),
        }
    }

    // /// Creates a new HTTPS client for a test.
    // fn https() -> TestClient<C> {
    //     let mut ca_cert_file = File::open("/source/tests/ssl/ca.der").unwrap();
    //     let mut ca_cert_buffer = Vec::new();
    //     ca_cert_file.read_to_end(&mut ca_cert_buffer).unwrap();

    //     let mut pkcs12_file = File::open("/source/tests/ssl/client.p12").unwrap();
    //     let mut pkcs12_buffer = Vec::new();
    //     pkcs12_file.read_to_end(&mut pkcs12_buffer).unwrap();

    //     let mut builder = TlsConnector::builder().unwrap();
    //     builder.add_root_certificate(Certificate::from_der(&ca_cert_buffer).unwrap()).unwrap();
    //     builder.identity(Pkcs12::from_der(&pkcs12_buffer, "secret").unwrap()).unwrap();

    //     let tls_connector = builder.build().unwrap();

    //     TestClient {
    //         c: Client::with_options(
    //             &["https://etcdsecure:2379"],
    //             ClientOptions {
    //                 tls_connector: Some(tls_connector),
    //                 username_and_password: None,
    //             },
    //         ).unwrap(),
    //     }
    // }
}

impl<C> Drop for TestClient<C> where C: Clone + Connect {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        let mut core = Core::new().unwrap();
        let work = kv::delete(&self.c, "/test", true);
        core.run(work).unwrap();
    }
}

impl<C> Deref for TestClient<C> where C: Clone + Connect {
    type Target = Client<C>;

    fn deref(&self) -> &Self::Target {
        &self.c
    }
}

#[test]
fn create() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let work = Box::new(kv::create(&client, "/test/foo", "bar", Some(60)).and_then(|response| {
        let node = response.node.unwrap();

        assert_eq!(response.action, "create");
        assert_eq!(node.value.unwrap(), "bar");
        assert_eq!(node.ttl.unwrap(), 60);

        Ok(())
    }));

    core.run(work).unwrap();
}

#[test]
fn create_does_not_replace_existing_key() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let fut_create = kv::create(&client, "/test/foo", "bar", Some(60));

    let work = fut_create.and_then(|_| {
        let fut_create = kv::create(&client, "/test/foo", "bar", Some(60));

        let work = fut_create.then(|result| {
            let errors = result.err().unwrap();

            for error in errors {
                match error {
                    Error::Api(ref error) => assert_eq!(error.message, "Key already exists"),
                    _ => panic!("expected EtcdError due to pre-existing key"),
                }
            }

            result
        });

        Box::new(work)
    });

    core.run(work).unwrap();
}

#[test]
fn create_in_order() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let keys: Vec<String> = (1..4).map(|ref _key| {
        client.create_in_order(
            "/test/foo",
            "bar",
            None,
        ).ok().unwrap().node.unwrap().key.unwrap()
    }).collect();

    assert!(keys[0] < keys[1]);
    assert!(keys[1] < keys[2]);
}

#[test]
fn create_in_order_must_operate_on_a_directory() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    assert!(client.create_in_order("/test/foo", "baz", None).is_err());
}

#[test]
fn compare_and_delete() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let modified_index = client.create(
        "/test/foo",
        "bar",
        None
    ).ok().unwrap().node.unwrap().modified_index.unwrap();

    let response = client.compare_and_delete(
        "/test/foo",
        Some("bar"),
        Some(modified_index)
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndDelete");
}

#[test]
fn compare_and_delete_only_index() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let modified_index = client.create(
        "/test/foo",
        "bar",
        None
    ).ok().unwrap().node.unwrap().modified_index.unwrap();

    let response = client.compare_and_delete(
        "/test/foo",
        None,
        Some(modified_index)
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndDelete");
}

#[test]
fn compare_and_delete_only_value() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    let response = client.compare_and_delete(
        "/test/foo",
        Some("bar"),
        None,
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndDelete");
}

#[test]
fn compare_and_delete_requires_conditions() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    for error in client.compare_and_delete("/test/foo", None, None).err().unwrap().iter() {
        match error {
            &Error::InvalidConditions(message) => assert_eq!(
                message,
                "Current value or modified index is required."
            ),
            _ => panic!("expected Error::InvalidConditions"),
        }
    }
}

#[test]
fn compare_and_swap() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let modified_index = client.create(
        "/test/foo",
        "bar",
        None
    ).ok().unwrap().node.unwrap().modified_index.unwrap();

    let response = client.compare_and_swap(
        "/test/foo",
        "baz",
        Some(100),
        Some("bar"),
        Some(modified_index)
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndSwap");
}

#[test]
fn compare_and_swap_only_index() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let modified_index = client.create(
        "/test/foo",
        "bar",
        None
    ).ok().unwrap().node.unwrap().modified_index.unwrap();

    let response = client.compare_and_swap(
        "/test/foo",
        "baz",
        None,
        None,
        Some(modified_index)
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndSwap");
}

#[test]
fn compare_and_swap_only_value() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    let response = client.compare_and_swap(
        "/test/foo",
        "bar",
        None,
        Some("bar"),
        None,
    ).ok().unwrap();

    assert_eq!(response.action, "compareAndSwap");
}

#[test]
fn compare_and_swap_requires_conditions() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    for error in client.compare_and_swap(
        "/test/foo",
        "bar",
        None,
        None,
        None,
    ).err().unwrap().iter() {
        match error {
            &Error::InvalidConditions(message) => assert_eq!(
                message,
                "Current value or modified index is required."
            ),
            _ => panic!("expected Error::InvalidConditions"),
        }
    }
}

#[test]
fn get() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", Some(60)).ok().unwrap();

    let response = client.get("/test/foo", false, false, false).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(response.action, "get");
    assert_eq!(node.value.unwrap(), "bar");
    assert_eq!(node.ttl.unwrap(), 60);

}

#[test]
fn get_non_recursive() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.set("/test/dir/baz", "blah", None).ok();
    client.set("/test/foo", "bar", None).ok();

    let response = client.get("/test", true, false, false).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(node.dir.unwrap(), true);

    let nodes = node.nodes.unwrap();

    assert_eq!(nodes[0].clone().key.unwrap(), "/test/dir");
    assert_eq!(nodes[0].clone().dir.unwrap(), true);
    assert_eq!(nodes[1].clone().key.unwrap(), "/test/foo");
    assert_eq!(nodes[1].clone().value.unwrap(), "bar");
}

#[test]
fn get_recursive() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.set("/test/dir/baz", "blah", None).ok();

    let response = client.get("/test", true, true, false).ok().unwrap();
    let nodes = response.node.unwrap().nodes.unwrap();

    assert_eq!(nodes[0].clone().nodes.unwrap()[0].clone().value.unwrap(), "blah");
}

// #[test]
// fn https() {
//     let client = TestClient::https();

//     if let Err(errors) = client.create("/test/foo", "bar", Some(60)) {
//         for error in errors {
//             println!("ERROR: {}", error);
//         }
//     }

//     let response = client.get("/test/foo", false, false, false).ok().unwrap();
//     let node = response.node.unwrap();

//     assert_eq!(response.action, "get");
//     assert_eq!(node.value.unwrap(), "bar");
//     assert_eq!(node.ttl.unwrap(), 60);
// }

// #[test]
// fn https_without_valid_client_certificate() {
//     let client = Client::new(&["https://etcdsecure:2379"]).unwrap();

//     assert!(client.get("/test/foo", false, false, false).is_err());
// }

#[test]
fn leader_stats() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.leader_stats().unwrap();
}

#[test]
fn self_stats() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    for result in client.self_stats().iter() {
        assert!(result.is_ok());
    }
}

#[test]
fn set() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let response = client.set("/test/foo", "baz", None).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(response.action, "set");
    assert_eq!(node.value.unwrap(), "baz");
    assert!(node.ttl.is_none());
}

#[test]
fn set_dir() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    assert!(client.set_dir("/test", None).is_ok());
    assert!(client.set_dir("/test", None).is_err());

    client.set("/test/foo", "bar", None).ok().unwrap();

    assert!(client.set_dir("/test/foo", None).is_ok());
}

#[test]
fn store_stats() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    for result in client.store_stats() {
        assert!(result.is_ok())
    }
}

#[test]
fn update() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    let response = client.update("/test/foo", "blah", Some(30)).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(response.action, "update");
    assert_eq!(node.value.unwrap(), "blah");
    assert_eq!(node.ttl.unwrap(), 30);
}

#[test]
fn update_requires_existing_key() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    for error in client.update("/test/foo", "bar", None).err().unwrap().iter() {
        match error {
            &Error::Api(ref error) => assert_eq!(error.message, "Key not found"),
            _ => panic!("expected EtcdError due to missing key"),
        }
    };
}

#[test]
fn update_dir() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create_dir("/test", None).ok().unwrap();

    let response = client.update_dir("/test", Some(60)).ok().unwrap();

    assert_eq!(response.node.unwrap().ttl.unwrap(), 60);
}

#[test]
fn update_dir_replaces_key() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.set("/test/foo", "bar", None).ok().unwrap();

    let response = client.update_dir("/test/foo", Some(60)).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(node.value.unwrap(), "");
    assert_eq!(node.ttl.unwrap(), 60);
}

#[test]
fn update_dir_requires_existing_dir() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    assert!(client.update_dir("/test", None).is_err());
}

#[test]
fn delete() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create("/test/foo", "bar", None).ok().unwrap();

    let response = client.delete("/test/foo", false).ok().unwrap();

    assert_eq!(response.action, "delete");
}

#[test]
fn create_dir() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    let response = client.create_dir("/test/dir", None).ok().unwrap();
    let node = response.node.unwrap();

    assert_eq!(response.action, "create");
    assert!(node.dir.is_some());
    assert!(node.value.is_none());
}

#[test]
fn delete_dir() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    client.create_dir("/test/dir", None).ok().unwrap();

    let response = client.delete_dir("/test/dir").ok().unwrap();

    assert_eq!(response.action, "delete");
}

// #[test]
// fn watch() {
//     let child = spawn(|| {
//         let client = Client::new(&["http://etcd:2379"]).unwrap();

//         sleep(Duration::from_millis(50));

//         client.set("/test/foo", "baz", None).ok().unwrap();
//     });

//     let client = TestClient::new();

//     client.create("/test/foo", "bar", None).ok().unwrap();

//     let response = client.watch("/test/foo", None, false).ok().unwrap();

//     assert_eq!(response.node.unwrap().value.unwrap(), "baz");

//     child.join().ok().unwrap();
// }

// #[test]
// fn watch_index() {
//     let client = TestClient::new();

//     let index = client.set("/test/foo", "bar", None).ok().unwrap().node.unwrap().modified_index.unwrap();

//     let response = client.watch("/test/foo", Some(index), false).ok().unwrap();
//     let node = response.node.unwrap();

//     assert_eq!(node.modified_index.unwrap(), index);
//     assert_eq!(node.value.unwrap(), "bar");
// }

// #[test]
// fn watch_recursive() {
//     let child = spawn(|| {
//         let client = Client::new(&["http://etcd:2379"]).unwrap();

//         sleep(Duration::from_millis(50));

//         client.set("/test/foo/bar", "baz", None).ok().unwrap();
//     });

//     let client = TestClient::new();

//     let response = client.watch("/test", None, true).ok().unwrap();
//     let node = response.node.unwrap();

//     assert_eq!(node.key.unwrap(), "/test/foo/bar");
//     assert_eq!(node.value.unwrap(), "baz");

//     child.join().ok().unwrap();
// }

#[test]
fn versions() {
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let client = TestClient::new(&handle);

    for result in client.versions().into_iter() {
        let version = result.ok().unwrap();

        assert_eq!(version.cluster_version, "2.3.0");
        assert_eq!(version.server_version, "2.3.7");
    }
}
