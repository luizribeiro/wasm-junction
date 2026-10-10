use super::super::test_support::store;
use super::*;

#[tokio::test]
async fn disabled_settings_refuse_connect_without_poisoning_store() {
    let mut store = store();
    let connector =
        HostTcpSocket::create(&mut views::sockets(store.data_mut()), IpAddressFamily::Ipv4)
            .unwrap();
    let connector_id = connector.rep();
    let connect = store
        .run_concurrent(async |accessor| {
            let accessor = accessor.with_getter::<WasiSockets>(WasiSocketsView::sockets);
            connect(
                &accessor,
                Resource::new_borrow(connector_id),
                IpSocketAddress::Ipv4(
                    wasmtime_wasi::p3::bindings::sockets::types::Ipv4SocketAddress {
                        port: 0,
                        address: (127, 0, 0, 1),
                    },
                ),
            )
            .await
        })
        .await
        .unwrap()
        .unwrap_err();
    assert!(connect.downcast_ref().is_none());
    assert_eq!(connect.to_string(), "sockets are disabled");
    assert_eq!(
        HostTcpSocket::get_address_family(
            &mut views::sockets(store.data_mut()),
            Resource::new_borrow(connector_id),
        )
        .unwrap(),
        IpAddressFamily::Ipv4
    );
}
