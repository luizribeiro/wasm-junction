package assistant

secret_paths := {"private/passwords.md"}
private_prefix := "private/"
drafts_prefix := "drafts/"
allowed_hosts := {
	"web-tools": {"*"},
	"weather-tools": {"api.open-meteo.com"},
}

deny contains sprintf("%s may not read notes", [input.event.caller]) if {
	reading(input.event)
	input.event.caller != "notes-tools"
}

deny contains sprintf("%s may not write notes", [input.event.caller]) if {
	writing(input.event)
	input.event.caller != "notes-tools"
}

deny contains sprintf("%s is secret", [path]) if {
	path := reading(input.event)
	path in secret_paths
}

deny contains sprintf("%s is outside %s", [path, drafts_prefix]) if {
	path := writing(input.event)
	not startswith(path, drafts_prefix)
}

deny contains sprintf("%s may not send to %s", [caller, host]) if {
	host := sending(input.event)
	caller := input.event.caller
	hosts := allowed_hosts[caller]
	not "*" in hosts
	not host in hosts
}

ask contains reason if {
	host := sending(input.event)
	not host in trusted_hosts
	some path in input.label.private
	reason := sprintf("send to %s with %s in the conversation", [host, path])
	not reason in input.approved
}

ask contains reason if {
	path := writing(input.event)
	some source in input.label.untrusted
	reason := sprintf("write %s with content from %s in the conversation", [path, source])
	not reason in input.approved
}
