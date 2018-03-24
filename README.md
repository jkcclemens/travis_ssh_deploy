# travis_ssh_deploy

Travis SSH Deploy is a flexible system to have Travis CI deploy to your server automatically without
using any third-party services.

In order to achieve this goal, SSH is used. Travis generates a SSH key for each repository it runs
builds for, and that key can be used to allow Travis access to your server.

Obviously, allowing Travis unrestricted access to a shell is a bad idea, so I whipped together this
program that you can limit it to.

The program accepts uploads and reads a configuration file on your server to determine what to do
with them.

## Building

Build Travis SSH Deploy's receiving end (the end that goes on your server) using `cargo`.

```rust
cargo build --release --bin receive
```

I would recommend you build the sending end and supply it to Travis somehow so that you can create
your deploy script using it.

```rust
cargo build --release --bin send
```

## Usage

Firstly, get your Travis repo's pubkey.

```sh
travis pubkey -r owned/repo
```

Add the pubkey to your `~/.ssh/authorized_keys` on your server like so.

    ...
    command="/path/to/receive /path/to/config.yaml" ssh-rsa AAAA...
    ...

Now, Travis will be able to SSH to your server, but it will always run the `receive` program.

To have it work, make Travis run the following commands.

```sh
/path/to/send your files here | ssh you@yourserver deploy deploy_plan_name
```

*Alternatively*, you can look at the `protocol` file in the repository and manually construct the
byte stream to send over SSH's stdin.

Be sure to add a deploy plan to your configuration file. See the file in the repository for an
example.
