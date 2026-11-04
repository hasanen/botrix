# Two-line prompt: user, git email, cwd and branch; input starts on its own line.
__prompt_git() {
  local branch email
  branch=$(git symbolic-ref --short -q HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null) || return
  email=$(git config user.email 2>/dev/null)
  printf ' (%s%s)' "$branch" "${email:+ · $email}"
}
PS1='\[\e[32m\]\u\[\e[0m\] \[\e[34m\]\w\[\e[33m\]$(__prompt_git)\[\e[0m\]\n\$ '
