# Contributing Guide

Thank you for contributing to luaskills. To keep every contribution traceable and properly licensed, this repository uses the DCO (Developer Certificate of Origin) as a lightweight contribution confirmation mechanism.

## DCO Requirement

This project is released under the MIT license. By contributing code to this repository, you confirm that you have the right to submit the changes and that you agree to license them under this project's MIT license.

Every contribution commit must include a `Signed-off-by` line. Please use the `-s` option when committing:

```bash
git commit -s -m "commit message"
```

The generated commit message should include a line like this:

```text
Signed-off-by: Your Name <your.email@example.com>
```

This means you confirm that the contribution is your original work, or that you have the right to submit it on behalf of the original author, and that it may be licensed under this project's license.

## Fixing Missing Sign-Offs

If only the latest commit is missing the sign-off line, run:

```bash
git commit --amend -s --no-edit
```

If multiple commits on a branch need to be signed, run:

```bash
git rebase --signoff main
```

After rewriting commits, update your remote branch with:

```bash
git push --force-with-lease
```

## Pull Request Requirements

Before opening a Pull Request, please make sure:

- Every commit includes a `Signed-off-by` line.
- The commit email belongs to an identity you are allowed to use.
- The contributed content may be licensed to this project under the MIT license.
- The change is focused and does not mix unrelated formatting or refactoring.

Pull Requests without DCO sign-off lines may be blocked until the commits are properly signed.

## Reference

See the standard DCO text here: [Developer Certificate of Origin](https://developercertificate.org/).

# 贡献指南

感谢你愿意为 luaskills 贡献代码。为了确保每一份贡献都有清晰的授权来源，本仓库采用 DCO（Developer Certificate of Origin，开发者原产地证明）作为轻量贡献确认机制。

## DCO 要求

本项目以 MIT 协议发布。向本仓库提交代码时，你需要确认你有权提交这些变更，并同意这些变更按照本项目的 MIT 协议授权。

所有贡献提交都必须带有 `Signed-off-by` 行。请在提交时使用 `-s` 参数：

```bash
git commit -s -m "提交说明"
```

生成的提交信息末尾应包含类似内容：

```text
Signed-off-by: Your Name <your.email@example.com>
```

这表示你确认该提交由你原创，或你有权代表原作者提交，并同意按本项目许可证授权。

## 忘记签署时的处理

如果只是最近一次提交忘记添加签署行，可以执行：

```bash
git commit --amend -s --no-edit
```

如果一个分支里有多次提交需要补签，可以执行：

```bash
git rebase --signoff main
```

补签后需要强制更新你的远端分支：

```bash
git push --force-with-lease
```

## Pull Request 要求

提交 Pull Request 前，请确认：

- 所有提交都包含 `Signed-off-by` 行。
- 提交邮箱与你有权使用的身份一致。
- 贡献内容可以按 MIT 协议授权给本项目。
- 变更范围聚焦，避免混入无关格式化或重构。

未包含 DCO 签署行的 Pull Request 可能会被要求补签后再合并。

## 参考

DCO 的标准说明可参考：[Developer Certificate of Origin](https://developercertificate.org/)。
