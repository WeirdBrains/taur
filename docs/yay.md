# Using with yay

Point yay at TAUR per invocation (keeps your real config untouched):

```sh
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Ss hello
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -Si hello-taur
yay --aururl http://127.0.0.1:8080 --aurrpcurl 'http://127.0.0.1:8080/rpc?' -G hello-taur
```

Verified with yay v13.0.1: search, info and `-G` (git clone) work. Quote the RPC URL in
zsh (`?` is a glob).
