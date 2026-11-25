vantage_input_test() {
    if [ "$(tty)" = /dev/tty1 ]; then
        /usr/local/bin/vantage-input
    fi
}
bind -x '"\eOP":vantage_input_test'
bind -x '"\e[[A":vantage_input_test'
