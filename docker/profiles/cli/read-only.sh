# Reads only. Enforced: the write scope is removed, whatever the container was given (write
# commands are refused with `readOnly`, nothing is sent).
unset POLARION_WRITE_PROJECTS POLARION_WRITE_TARGETS POLARION_ALLOW_GLOBAL_WRITES
# Write flags given as arguments (--write-projects, --write-targets, --allow-global-writes) are
# the caller's choice and still apply.
