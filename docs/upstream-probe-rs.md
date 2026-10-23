# CW32L0 与 probe-rs 的上游支持（结论：已内置，无需提 PR）

## 现状

probe-rs **0.32.0 已经内置 CW32L0 支持**，不需要我们提交任何东西：

```console
$ probe-rs chip list | grep -A3 CW32
CW32L0 Series
        CW32L010x8
        CW32L011x8
        CW32L012x8
```

```console
$ probe-rs download --chip CW32L012x8 --protocol swd <elf>
     Finished in 0.81s
```

也就是说：烧录、复位、`probe-rs run`、VS Code 调试（`probe-rs-debug` 扩展）
全都可以直接写 `"chip": "CW32L012x8"`，**不需要 `chipDescriptionPath`**。

> 唯一容易踩的坑：**必须显式 `--protocol swd` / `"wireProtocol": "Swd"`**。
> 不写的话 probe-rs 默认走 JTAG，报 `JtagGetIdcodeError`，很容易被误判成
> "不支持这颗芯片"。

## 那 `tools/CW32L0_Series.yaml` 还有用吗

有，两个场景：

1. probe-rs **< 0.32** 不认识 CW32L0，要用
   `--chip-description-path tools/CW32L0_Series.yaml`（VS Code 里是
   `"chipDescriptionPath"`）才能工作；
2. 厂商出了新封装/新 Flash 容量时，可以用厂商 DFP 重新生成一份覆盖。

这份 YAML 由 `target-gen` 从 `WHXY.CW32L012_DFP` 1.0.2 直接生成
（`generated_from_pack: true`），没有手改，格式与上游 targets 一致。

## 如果以后要加别的芯片系列

流程（probe-rs 官方要求）：

1. `cargo install target-gen`
2. `target-gen pack <厂商 DFP .pack> ./out` —— **必须是 target-gen 生成，不能手写**
3. 把 `out/<Family>.yaml` 放到 fork 的 `probe-rs/targets/` 下
4. `CHANGELOG.md` 的 *Added* 加一行
5. 开 PR；描述里写清：pack 名称与版本、生成命令、实测过的板子和探针

维护者只看 GitHub issue/PR，不看邮件。

## 本次排查时间线（供以后参考）

1. 一开始以为不支持 → 用 YAML + `--chip-description-path` 绕过去，工作正常
2. VS Code 里报 `Unable to open probe: No core at the specified index.`
3. 查 `probe-rs chip list` 才发现 **0.32 早就内置了**；`download --chip CW32L012x8`
   不带任何 YAML 也能烧
4. 结论：launch.json 里的 `chipDescriptionPath` 是多余的，去掉后 DAP 实测
   （`tools/dap_check.py`）launch 成功、烧录完成、调用栈落在 `main`
