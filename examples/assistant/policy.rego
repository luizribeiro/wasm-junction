package assistant

secret_paths := {"private/passwords.md"}
private_prefix := "private/"
drafts_prefix := "drafts/"
allowed_hosts := {
	"agent": {data.model_host},
	"web-tools": {"*"},
	"weather-tools": {"api.open-meteo.com"},
	"notes-tools": set(),
}

deny contains sprintf("%s may not read notes", [request.caller]) if {
	reading(request)
	request.caller != "notes-tools"
}

deny contains sprintf("%s may not write notes", [request.caller]) if {
	writing(request)
	request.caller != "notes-tools"
}

deny contains sprintf("%s is secret", [request.path]) if {
	reading(request)
	request.path in secret_paths
}

deny contains sprintf("%s is outside %s", [request.path, drafts_prefix]) if {
	writing(request)
	not starts_with_drafts(request.path)
}

deny contains sprintf("%s may not send to %s", [request.caller, request.host]) if {
	request.action == "http.request"
	not host_allowed(request.caller, request.host)
}

host_allowed(caller, host) if "*" in allowed_hosts[caller]
host_allowed(caller, host) if host in allowed_hosts[caller]

private_data_host(item) if {
	item.action == "http.request"
	item.host in private_data_hosts
}

ask contains reason if {
	request.action == "http.request"
	not private_data_host(request)
	some path in input.label.private
	reason := sprintf("conversation includes %s", [path])
}

ask contains reason if {
	writing(request)
	some source in input.label.untrusted
	reason := sprintf("conversation includes content from %s", [source])
}

consent_key(item, reason) := sprintf("%s|%s|%s", [item.caller, effect(item), reason])

deny contains reason if {
	some reason in ask
	consent_key(request, reason) in input.refused
}
