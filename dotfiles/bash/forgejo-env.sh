# Source from ~/.bashrc to expose FORGEJO_TOKEN without storing the token value
# in shell startup files.

forgejo_token_file="$HOME/Documents/Programming/forgejo-migration/.forgejo-admin-token"
if [ -z "${FORGEJO_TOKEN:-}" ] && [ -r "$forgejo_token_file" ]; then
    export FORGEJO_TOKEN="$(tr -d '\r\n' < "$forgejo_token_file")"
fi
unset forgejo_token_file

