#[allow(
    clippy::wildcard_imports,
    reason = "request context shares the parent HTTP value machinery"
)]
use super::*;

pub(super) type Headers = Vec<(String, Vec<u8>)>;
pub(super) type RequestContext = (
    Method,
    Option<Scheme>,
    Option<String>,
    Option<String>,
    Headers,
);

pub(super) fn read(
    store: &mut StoreData,
    request: &Resource<HostOutgoingRequest>,
) -> wasmtime::Result<RequestContext> {
    let id = request.rep();
    let mut view = views::http(store);
    let fields = OutgoingRequestApi::headers(&mut view, Resource::new_borrow(id))?;
    let context = (
        OutgoingRequestApi::method(&mut view, Resource::new_borrow(id))?,
        OutgoingRequestApi::scheme(&mut view, Resource::new_borrow(id))?,
        OutgoingRequestApi::authority(&mut view, Resource::new_borrow(id))?,
        OutgoingRequestApi::path_with_query(&mut view, Resource::new_borrow(id))?,
        HostFields::entries(&mut view, Resource::new_borrow(fields.rep()))?,
    );
    HostFields::drop(&mut view, fields)?;
    Ok(context)
}

pub(super) fn apply(
    store: &mut StoreData,
    request: &Resource<HostOutgoingRequest>,
    context: RequestContext,
) -> wasmtime::Result<()> {
    let (method, scheme, authority, path, headers) = context;
    let id = request.rep();
    let mut view = views::http(store);
    let valid = OutgoingRequestApi::set_method(&mut view, Resource::new_borrow(id), method)?
        .and(OutgoingRequestApi::set_scheme(
            &mut view,
            Resource::new_borrow(id),
            scheme,
        )?)
        .and(OutgoingRequestApi::set_authority(
            &mut view,
            Resource::new_borrow(id),
            authority,
        )?)
        .and(OutgoingRequestApi::set_path_with_query(
            &mut view,
            Resource::new_borrow(id),
            path,
        )?);
    if valid.is_err() {
        return Err(wasmtime::Error::msg(
            "middleware produced invalid HTTP request metadata",
        ));
    }
    let fields = HostFields::from_list(&mut view, headers)
        .map_err(|error| wasmtime::Error::msg(error.to_string()))?;
    let fields = view.table.delete(fields)?;
    view.table
        .get_mut(&Resource::<HostOutgoingRequest>::new_borrow(id))?
        .headers = fields;
    Ok(())
}

pub(super) fn value(context: RequestContext) -> Val {
    Val::Tuple(vec![
        context.0.to_http_val(),
        context.1.to_http_val(),
        context.2.to_http_val(),
        context.3.to_http_val(),
        context.4.to_http_val(),
    ])
}
