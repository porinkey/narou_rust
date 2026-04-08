<p align="center">
  <img src="./assets/logo.svg" alt="narou_rust logo" width="920">
</p>

<p align="center">
  <strong>Fast CLI downloader and EPUB builder for web novels.</strong>
</p>

<p align="center">
  Download • Update • Repair • Convert • EPUB
</p>

`narou_rust` は、Web 小説をダウンロードしてローカル保存し、必要に応じて EPUB まで生成できる Rust 製の CLI ツールです。

現在は次のような用途を想定しています。

- 小説家になろう系サイトの作品を保存する
- カクヨム作品を保存する
- 保存済み作品を更新する
- 一覧や診断、修復を CLI で行う
- AozoraEpub3 を使って EPUB を作る

CLI と対話モードに特化しています。

## 特徴

- Rust 製で軽快に動作
- 引数付き CLI と、引数なしの対話モードの両方に対応
- ダウンロード、更新、一覧、再変換、診断、修復をひとつのツールで完結
- `error_report:` 形式の構造化エラー出力に対応
- `doctor` / `repair` で保存データの状態確認と復旧が可能
- `--epub` でダウンロード後にそのまま EPUB を生成可能

## 対応サイト

- `https://ncode.syosetu.com/`
- `https://novel18.syosetu.com/`
- `https://noc.syosetu.com/`
- `https://mnlt.syosetu.com/`
- `https://mid.syosetu.com/`
- `https://kakuyomu.jp/`

補足:

- `n2839il` のような N コード単体指定は `ncode.syosetu.com` 向けです
- R18 系サイトは URL 指定を推奨します
- カクヨムは URL 指定を使ってください

## できること

- `download`
- `update`
- `batch-download`
- `list`
- `convert`
- `inspect`
- `remove`
- `doctor`
- `repair`
- 引数なし起動の対話モード

## 必要環境

### 必須

- Windows
- Rust stable
- `cargo`
- Java

### EPUB 生成を使う場合

- `AozoraEpub3-1.1.1b30Q`
- `AozoraEpub3.jar`

既定では、次のどちらかに `AozoraEpub3-1.1.1b30Q` ディレクトリがあることを想定しています。

- workspace 直下
- workspace の親ディレクトリ直下

別の場所に置く場合は `--aozora-dir` を指定してください。

## インストール

### 1. リポジトリを取得

```powershell
git clone <YOUR_REPOSITORY_URL>
cd narou_rust
```

### 2. ビルド

```powershell
cargo build -p narou_rust_cli --release
```

実行ファイル:

```text
target\release\narou_rust_cli.exe
```

開発中に試すだけなら `--release` なしでも使えます。

## Quick Start

最短で試すならこの 3 ステップです。

### 1. ビルド

```powershell
git clone <YOUR_REPOSITORY_URL>
cd narou_rust
cargo build -p narou_rust_cli --release
```

### 2. 1 作品ダウンロード

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  download https://ncode.syosetu.com/n2839il/
```

### 3. EPUB も同時に作る

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  --epub `
  --aozora-dir D:\tools\AozoraEpub3-1.1.1b30Q `
  download https://ncode.syosetu.com/n2839il/
```

出力先の例:

```text
D:\work\narou_data\
  .narou\
    database.yaml
  小説データ\
    小説家になろう\
      n2839il 作品名\
        toc.yaml
        本文\
        raw\
        n2839il 作品名.txt
        [作者名] 作品名.epub
```

### 対話モードで使う

```powershell
.\target\release\narou_rust_cli.exe
```

引数なしで起動すると、メニュー形式で次の操作ができます。

- ダウンロード
- 更新
- 一括ダウンロード
- 保存済み作品一覧
- 保存済み作品の再変換
- 保存済み作品の詳細表示
- 保存済み作品の削除
- `doctor`
- `repair`

## 基本的な使い方

`--workspace` で保存先ルートを切り替えられます。省略時はカレントディレクトリを使います。

### ダウンロード

なろう作品:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  download n2839il
```

R18 系:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_r18 `
  download https://novel18.syosetu.com/n1610bw/
```

カクヨム:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\kakuyomu_data `
  download https://kakuyomu.jp/works/4852201425154905871
```

`raw/*.html` が不要なら `--no-raw` を付けます。

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  download n2839il --no-raw
```

### 更新

workspace 内の全作品を更新:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  update
```

特定作品だけ更新:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  update n2839il
```

複数指定:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  update n2839il https://ncode.syosetu.com/n9669bk/
```

### 一括ダウンロード

入力ファイル例:

```text
https://ncode.syosetu.com/n2839il/
n9669bk
# comment
https://kakuyomu.jp/works/4852201425154905871
```

実行:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_batch `
  batch-download D:\work\urls.txt
```

失敗が 1 件でもある場合、終了コードは `1` になります。

### 一覧表示

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  list
```

サイトで絞る:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  list --site カクヨム
```

タイトル、作者、URL で検索:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  list --query 横浜
```

詳細表示:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  list --verbose
```

JSON 出力:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  list --json
```

### EPUB 再生成

保存済み作品から `txt` と `epub` を再生成します。

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  --aozora-dir D:\tools\AozoraEpub3-1.1.1b30Q `
  --epub `
  convert 0
```

### 詳細確認

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  inspect 0
```

### 削除

データベースから削除:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  remove 0
```

作品フォルダも削除:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  remove 0 --files
```

## EPUB 生成

`--epub` を付けると、`download` / `update` / `batch-download` / `convert` 実行時に EPUB を生成します。

### ダウンロードと同時に生成

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_epub `
  --epub `
  --aozora-dir D:\tools\AozoraEpub3-1.1.1b30Q `
  download https://ncode.syosetu.com/n2839il/
```

### 更新と同時に生成

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_epub `
  --epub `
  --aozora-dir D:\tools\AozoraEpub3-1.1.1b30Q `
  update
```

### バッチダウンロードと同時に生成

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_epub `
  --epub `
  --aozora-dir D:\tools\AozoraEpub3-1.1.1b30Q `
  batch-download D:\work\urls.txt
```

## 保存されるファイル

例:

```text
<workspace>\
  .narou\
    database.yaml
    local_setting.yaml
  小説データ\
    小説家になろう\
      <作品フォルダ>\
        toc.yaml
        raw\
          1 <話名>.html
        本文\
          1 <話名>.yaml
        <作品フォルダ>.txt
        <author title>.epub
```

主なファイル:

- `.narou/database.yaml`
  - 保存済み作品の一覧
- `toc.yaml`
  - タイトル、作者、あらすじ、目次
- `raw/*.html`
  - 取得した元 HTML
- `本文/*.yaml`
  - 前書き、本文、後書きの分割済みデータ
- `*.txt`
  - AozoraEpub3 に渡す青空文庫テキスト
- `*.epub`
  - 生成された EPUB

## 設定ファイル

次の 2 つを読み込みます。

- `<workspace>\.narou\local_setting.yaml`
- `~\.narousetting\global_setting.yaml`

ローカル設定が優先されます。

設定例:

```yaml
download:
  interval: 0.5
  wait-steps: 20
  retry-limit: 3
  retry-wait-seconds: 2
  long-wait-seconds: 10
update:
  interval: 0.3
```

意味:

- `download.interval`
  - 各 HTTP リクエスト前の待機秒数
- `download.wait-steps`
  - 指定回数ごとに長めの待機を入れる
- `download.retry-limit`
  - 一時的なエラー時の再試行回数
- `download.retry-wait-seconds`
  - 再試行前の待機秒数
- `download.long-wait-seconds`
  - `wait-steps` 到達時の待機秒数
- `update.interval`
  - 複数作品更新時の作品間待機秒数

## `doctor`

`doctor` は、実行環境と保存データの状態を診断します。

確認内容:

- Java
- AozoraEpub3
- workspace
- 設定ファイル
- 保存済み件数
- `database.yaml`
- `toc.yaml`
- 本文ファイル不足
- 孤立フォルダ

実行例:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor
```

特定 ID:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor 0 3 5
```

サイト絞り込み:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor --site カクヨム
```

検索絞り込み:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor --query 横浜
```

JSON:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor --json
```

ログ保存:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  doctor --log-file D:\work\logs\doctor.txt
```

終了コード:

- `0`: 問題なし
- `2`: warning あり
- `3`: error あり

## `repair`

`repair` は `doctor` の結果を元に、修復できる問題を自動で直します。

対応内容:

- `toc.yaml` や本文不足がある作品の `update`
- `txt` や `epub` が不足している作品の `convert`
- `--prune` 指定時の孤立フォルダ削除

実行例:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair
```

特定 ID:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair 0 3 5
```

サイト絞り込み:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --site カクヨム
```

検索絞り込み:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --query 横浜
```

Dry run:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --dry-run
```

孤立フォルダ削除:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --prune
```

JSON:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --json
```

ログ保存:

```powershell
.\target\release\narou_rust_cli.exe `
  --workspace D:\work\narou_data `
  repair --log-file D:\work\logs\repair.txt
```

終了コード:

- `0`: 未解決 issue なし
- `2`: warning のみ残存
- `3`: error が残存

## `batch-download` の出力

`batch-download` 実行後、workspace 直下に次のファイルが生成されます。

- `batch_download_success.txt`
- `batch_download_failed.txt`
- `batch_download_summary.txt`

例:

```text
input_file: D:\work\urls.txt
total: 2
success: 1
failed: 1
success_file: D:\work\narou_batch\batch_download_success.txt
failed_file: D:\work\narou_batch\batch_download_failed.txt
```

## エラー出力

エラー時は `error_report:` 形式で出力します。

```text
error_report:
  code: invalid_target.url_missing_ncode
  stage: input
  summary: "command=download target=https://ncode.syosetu.com/invalid/ workspace=D:\\work\\narou"
  command: "cli"
  workspace: "D:\\work\\narou"
  causes:
    - "ncode not found in url: https://ncode.syosetu.com/invalid/"
  hints:
    - "入力URLに n1234ab の形式のNコードが含まれているか確認する"
    - "短縮URLや作品情報URLではなく作品本文/目次URLを使う"
```

この形式は、ログ収集や AI への相談時にも扱いやすい出力です。

## よく使うコマンド

ヘルプ:

```powershell
.\target\release\narou_rust_cli.exe --help
```

サブコマンドごとのヘルプ:

```powershell
.\target\release\narou_rust_cli.exe download --help
.\target\release\narou_rust_cli.exe update --help
.\target\release\narou_rust_cli.exe doctor --help
```

## 開発

テスト:

```powershell
cargo test -p narou_rust_core
```

CLI ビルド:

```powershell
cargo build -p narou_rust_cli --release
```

## License

This project is licensed under the MIT License.
