package assistant

secret_paths := {"private/passwords.md"}
private_prefix := "private/"
notes_prefix := "notes/"

deny contains sprintf("%s is secret", [path]) if {
	path := reading(input.event)
	path in secret_paths
}

deny contains sprintf("%s is outside %s", [path, notes_prefix]) if {
	path := writing(input.event)
	not startswith(path, notes_prefix)
}

ask contains reason if {
	host := sending(input.event)
	not host in trusted_hosts
	some path in private_reads
	reason := sprintf("send to %s after reading %s", [host, path])
	not reason in input.approved
}

ask contains reason if {
	path := writing(input.event)
	some host in web_fetches
	reason := sprintf("write %s after fetching from %s", [path, host])
	not reason in input.approved
}
