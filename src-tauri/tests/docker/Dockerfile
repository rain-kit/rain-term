# 集成测试用的 OpenSSH 服务器：用户 tester，密码 pa55；带 vim、top、htop 用来检查终端显示。
FROM alpine:3.22
RUN apk add --no-cache openssh-server openssh-sftp-server bash vim htop procps-ng ncurses-terminfo-base \
    && adduser -D -s /bin/bash tester \
    && echo 'tester:pa55' | chpasswd \
    && install -d -m 700 -o tester -g tester /home/tester/.ssh \
    && install -m 600 -o tester -g tester /dev/null /home/tester/.ssh/authorized_keys
COPY sshd_config /etc/ssh/sshd_config
EXPOSE 22
# 每个容器启动时生成自己的主机密钥，跳板机和目标机的指纹不同。
CMD ["sh", "-c", "ssh-keygen -A && exec /usr/sbin/sshd -D -e"]
