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

#[tokio::test]
async fn disabled_settings_refuse_listen_without_poisoning_store() {
    let mut store = store();
    let listener =
        HostTcpSocket::create(&mut views::sockets(store.data_mut()), IpAddressFamily::Ipv4)
            .unwrap();
    let address = IpSocketAddress::Ipv4(
        wasmtime_wasi::p3::bindings::sockets::types::Ipv4SocketAddress {
            port: 0,
            address: (127, 0, 0, 1),
        },
    );
    HostTcpSocket::bind(
        &mut views::sockets(store.data_mut()),
        Resource::new_borrow(listener.rep()),
        address,
    )
    .await
    .unwrap();
    let invocation = store.data().context.invocation_id().unwrap();
    let args = scope_values(
        vec![resource_to_val(
            &Resource::<TcpSocket>::new_borrow(listener.rep()),
            INTERFACE,
            TCP_SOCKET,
        )],
        invocation,
    );

    let error = listen_real(store.as_context_mut(), args).await.unwrap_err();
    assert_eq!(error.kind(), CallErrorKind::Refused);
    assert!(
        !HostTcpSocket::get_is_listening(
            &mut views::sockets(store.data_mut()),
            Resource::new_borrow(listener.rep()),
        )
        .unwrap()
    );
}
