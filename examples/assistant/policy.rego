package assistant

secret_paths := {"private/passwords.md"}
private_dir := "private"
drafts_dir := "drafts"
allowed_hosts := {
	"agent": {data.model_host},
	"web-tools": {"*"},
	"weather-tools": {"api.open-meteo.com"},
	"notes-tools": set(),
}
tool_calls := {
	"host": {"agent.answer"},
	"agent": {
		"notes-tools.list-notes",
		"notes-tools.read-note",
		"notes-tools.write-note",
		"web-tools.fetch",
		"weather-tools.get-weather",
	},
}

default private_data_hosts := set()
private_data_hosts := data.private_data_hosts

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

deny contains sprintf("%s is outside %s/", [request.path, drafts_dir]) if {
	writing(request)
	not under(request.path, drafts_dir)
}

deny contains sprintf("%s may not send to %s", [request.caller, request.host]) if {
	request.action == "http.request"
	not host_allowed(request.caller, request.host)
}

deny contains sprintf("%s may not call %s", [request.caller, request.tool]) if {
	request.action == "tool.call"
	not request.tool in tool_calls[request.caller]
}

deny contains sprintf("%s is not allowed by the policy", [request.call]) if {
	request.action == "other"
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

turn_private contains path if {
	some earlier in history_requests
	reading(earlier)
	path := earlier.path
	under(path, private_dir)
}

turn_private contains path if {
	some earlier in history_requests
	reading(earlier)
	path := earlier.path
	under(path, drafts_dir)
}

turn_untrusted contains source if {
	some earlier in history_requests
	reading(earlier)
	under(earlier.path, drafts_dir)
	source := earlier.path
}

turn_untrusted contains host if {
	some earlier in history_requests
	earlier.action == "http.request"
	earlier.caller == "web-tools"
	host := earlier.host
}

turn_untrusted contains "weather-tools" if {
	some earlier in history_requests
	earlier.action == "http.request"
	earlier.caller == "weather-tools"
}

turn_label := {"private": turn_private, "untrusted": turn_untrusted}

remember(item) := true if {
	item.caller == "agent"
	item.action == "http.request"
	item.host == data.model_host
} else := false

under(path, directory) if path == directory
under(path, directory) if startswith(path, sprintf("%s/", [directory]))
