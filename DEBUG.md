# マイクUSB再接続後の録音停止：調査報告

調査日：2026-09-22。対象コミット：`cc526641698a5f5c1b26ac89d5958e06c21c65c8`。
提示ログの採取日は未記載。同じログが2回掲載されているため、独立した2回の再現ではなく1回の観測として扱う。

更新：本報告に基づくローカル修正を実施した。以下の調査本文・行番号は上記コミット時点の記録であり、修正後の状態ではない。実装・検証結果は末尾の「修正実施」に記載する。実機の再接続成功はまだ確認していない。

**追加更新：22:18〜22:19のMIC2ログで、停止箇所が`EPENA=0`／`DIEPTSIZ=0`のFIFO空き待ちと判明した。前回修正だけでは未解決だった。末尾の「MIC2実測後の追加修正」が最新の判断・実装である。**

## 結論

**再接続後、EP0の診断要求には応答しているが、マイクEP0x81のFIFO投入回数とXFRC完了の計測値が停止している。最有力は、OTGドライバの未完了IN転送の復旧、またはReset／Alt切替時のキャンセル処理の問題である。**

ただし、現在のログだけで「EPENAが残った」「FIFO待ちで停止した」「Alt 1の重複設定が発火条件だった」のいずれかに断定することはできない。ホストが再接続後に録音用Isochronous URBを再開していない場合も、予約済みパケットが消費されず同様のカウンタ停止になり得る。

したがって、**マイクUSB転送の進行停止は確認済み、具体的な根本原因は未確定**である。以下ではログで確定した事実、コードで確認した問題候補、追加観測を区別する。

## 再現操作と観測

1. dick mouseを接続する。
2. Discordを開いて録音を開始する。音声が入る。
3. USBケーブルを抜き差しする。
4. 再度録音を開始する。音声が入らない。

代表的なログ抜粋：

```text
08:10:07 010 ep=0x81 alt=1 queued=38185 xfrc=38170 dqueued=1004 dxfrc=1004 dt=1.004s resets=6 alt0=23 alt1=17
08:10:14 010 ep=0x81 alt=1 queued=45217 xfrc=45202 dqueued=1004 dxfrc=1004 dt=1.004s resets=6 alt0=23 alt1=17
08:10:15 unavailable: DXXK c0de:0001 is not connected
08:10:16 unavailable: DXXK c0de:0001 is not connected
08:10:17 011 ep=0x81 alt=1 queued=46019 xfrc=46000 baseline resets=8 alt0=30 alt1=22
08:10:18 011 ep=0x81 alt=1 queued=46019 xfrc=46000 dqueued=0 dxfrc=0 dt=1.004s resets=8 alt0=30 alt1=22
08:10:35 011 ep=0x81 alt=1 queued=46019 xfrc=46000 dqueued=0 dxfrc=0 dt=1.004s resets=8 alt0=30 alt1=22
```

| 観測項目 | 再接続前 | 再接続後 | 判断 |
|---|---|---|---|
| USBアドレス | 010 | 011 | 再列挙後の別アドレスから診断を取得している |
| 最後に通知されたAlt | 1 | 1 | ソフトウェア上は録音用Altが選択されている |
| FIFO投入の増加率 | 約1,000回/秒 | 0 | 新しいマイクパケットを投入できていない |
| XFRC計測値の増加率 | 約1,000回/秒 | 0 | 新しい転送完了が計数されていない |
| EP0診断取得 | 成功 | 成功 | USB全体やEP0が完全停止した状態ではない |
| reset通知の累積 | 6 | 8で一定 | 再接続時に増加したが、その後リセットループは観測されない |

完全な行がある08:10:17〜08:10:35の約18秒間、両転送カウンタは一定である。08:10:36の行も同値だが、末尾が省略されている。

08:10:14→08:10:17の差は`queued +802`、`xfrc +798`。この区間には切断前の未観測時間と再列挙処理の両方が含まれるため、「再接続後に802パケット送れた」とは解釈できない。

## カウンタの意味と解釈上の制限

- **`queued`**：write呼び出し回数ではない。[OTGドライバの1879行](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1879)で、転送サイズ設定・EPENA設定・FIFOへのデータ投入が済んだ後に増える。戻り値が成功でも、その時点でホスト受信は完了していない。
- **`xfrc`**：[IN割り込み処理の135行](vendor/embassy-usb-synopsys-otg/src/lib.rs#L135)で`DIEPINT.XFRC`を読み、W1Cによるクリア前に増える。TXFE、EPDISD、incomplete-ISOの再予約は含めない。厳密には「ISRが観測したXFRC回数」であり、PCMの非ゼロ長受信や音声内容の正常性を保証しない。
- **`alt`**：[診断ハンドラの51行](dick_mouse/src/tasks/usb_diagnostics.rs#L51)が保存する最後の通知値。`DIEPCTL.USBAEP`、`EPENA`、NAK状態を読み出した値ではない。
- **`resets`**：[診断ハンドラの40行](dick_mouse/src/tasks/usb_diagnostics.rs#L40)で増えるUSB reset通知回数。MCU再起動回数ではない。
- **`alt0`／`alt1`**：該当インターフェースに対する通知回数。Reset時にもupstreamがAlt 0を通知するため、すべてがホストからの独立した`SET_INTERFACE`要求とは限らない。
- **`baseline`**：[読取スクリプトの106行](scripts/microphone_usb_stats.py#L106)がアドレス・endpoint・reset回数の変化で差分基準を取り直したことを示す。デバイス側カウンタをリセットした意味ではない。

カウンタはUSB resetやAlt切替で初期化されない。`queued−xfrc`は正常時にも14〜15程度あり、停止後は19である。この値には過去にキャンセル／flushされた予約や未計数の完了が含まれ得るため、**現在19パケットが待機している、または19パケットが今回失われたとは言えない**。

再接続前後で累積値が引き継がれていることは、MCUが給電を保ったままUSBだけ再接続された状況と整合する。ただし、給電構成自体はこのログでは確認できない。

## コードで確認した原因候補

依存関係はEmbassyの`cd7570483a7036f23a9925a339152396bec4c041`に固定され、OTGはローカルの`vendor/embassy-usb-synopsys-otg`を参照している。[Cargo設定](dick_mouse/Cargo.toml#L10)とlockfileを確認した。

vendored OTGと同リビジョンのローカルupstreamソースを比較した結果、`src/lib.rs`の差分は診断カウンタと読出し関数の追加だった。以下の動作は今回のカウンタ追加で導入したものではなく、固定版にも存在する。[パッチ説明](vendor/embassy-usb-synopsys-otg/DXXK_PATCH.md)、[対象upstream](https://github.com/embassy-rs/embassy/blob/cd7570483a7036f23a9925a339152396bec4c041/embassy-usb-synopsys-otg/src/lib.rs)。

### 1. 前回IN転送が終わらないと、次のwriteを永久に待たせる

**コード上確認済み。今回の停止箇所として有力だが、EPENAの実測はない。**

[1779行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1779)の`EndpointIn::write()`は、RAM上の`in_enabled`を確認し、`DIEPCTL.EPENA == 0`になるまで`Pending`を返す。タイムアウトや接続世代の判定はない。

[812行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L812)の`abort_in_endpoint()`はSNAK→INEPNE待ち→EPDIS→EPDISD待ちを行うが、待ちがタイムアウトしても警告を出すだけで、失敗を呼び出し元へ返さない。[Reset時のendpoint再設定](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1162)もこの関数を利用している。

このため、停止処理が成功しなかった場合でも再設定を続行できる。`EPENA`が残存し、正常なXFRCも発生しない状態になれば、`queued`と`xfrc`が両方止まる今回の観測と一致する。ただし、今回abortがタイムアウトした証拠はまだない。

[187行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L187)のincomplete-ISO処理も同じabortを利用し、FIFO内容を維持してフレーム偶奇を切り替え、再有効化する。再試行回数や復旧失敗をアプリケーションに返す経路がないため、ここも追加観測対象になる。

### 2. FIFO待機中にReset／Alt 0を処理しても、旧writeをキャンセルしない経路がある

**キャンセル確認の欠落はコード上確認済み。今回その待機に入ったかは未確認。**

[1804行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1804)のFIFO空き待ちは`DTXFSTS`だけを確認し、`in_enabled`を再確認しない。[1833行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1833)の実際の転送予約直前にも確認がない。

FIFO待機で一度`Pending`になった後にReset／Alt 0が処理されても、再poll時に`Disabled`を返さず、旧接続のwriteを続け得る。また、状態が`false→true`になってから再pollされた場合、その途中の無効化をboolだけでは識別できない。wakerによる通知は接続世代の記録にはならない。

[アプリケーションの249行](dick_mouse/src/tasks/usb.rs#L249)はwriteが`Err`を返したときにだけストリーミング状態を落とし、リングを破棄して開始待ちへ戻る。古いwriteが`Pending`のままなら、この復旧処理にも到達しない。

### 3. 転送中のAlt 1再設定で、abortせずFIFOをflushできる

**該当経路はコード上確認済み。今回Alt 1が重複設定されたかは未確認。**

[1538行以降](vendor/embassy-usb-synopsys-otg/src/lib.rs#L1538)のIN側`endpoint_set_enabled()`では、進行中の転送をabortする条件が`!enabled && EPENA`に限定されている。一方、FIFOのflushは`enabled=true`の場合も実行する。

upstreamの`SET_INTERFACE`処理は、同じAltを選び直した場合も`endpoint_set_enabled()`を呼ぶ。このため、転送中にAlt 1を再設定すると、EPENAを残したままSNAKを設定してFIFOを消す経路がある。転送長・有効状態・FIFO内容が不整合になり、次のwriteが前回転送待ちに入る可能性がある。

今回の`alt1=17→22`は、Alt 0→1を5回行った場合にも増える。**この累積値から「Alt 1→1が発生した」とは判断しない。** 確認にはEP0要求の順序が必要である。

## 今回の説明として優先度が低いもの

- **I²Sの無音、マイクゲイン、リング不足だけが原因という説明**：[マイク送信処理](dick_mouse/src/tasks/usb.rs#L234)はSPSCリングを非ブロッキングで読み、空でも減衰したサンプルを使ってパケットを生成する。I²Sが止まったりミュートされたりしても、USB転送が正常なら`queued`は増える。リング増量では今回の転送停止を直接解消できない。
- **EP0を含むUSB全体の停止**：診断値は[毎回のEP0 vendor IN要求](scripts/microphone_usb_stats.py#L47)で取得しており、キャッシュ表示ではない。停止後も応答しているため、以前のEP0 timeoutと同じ状態とは判断しない。
- **単純なwake通知の取りこぼしだけ**：EP0処理とマイク送信は[同じUSBタスク内のjoin](dick_mouse/src/tasks/usb.rs#L153)で実行される。定期的なEP0応答が続く状況では、マイクだけが一度wakeを失ったという説明より、再pollしても待機条件が解除されない可能性を優先する。
- **固定フォーマットの記述不備だけ**：現行マイクは48 kHz・mono S16・単一Asynchronous IN・最大98 bytes・feedbackなし・sample-rate controlなし。同一コードで接続直後は動作するため、今回は[descriptor](vendor/embassy-usb/src/class/uac1/source.rs#L69)変更より再接続時の状態遷移を優先する。

一方、**Discord／PipeWire／ALSAが再接続後に録音URBを再開していない可能性は残る**。Alt選択の成功は、ホストが継続してIN転送を要求している証拠ではない。カウンタだけを根拠にホスト側を除外しない。

## 根本原因を確定する追加検証

以下は次の検証仕様であり、この報告作成ではファームウェアや診断プロトコルを変更していない。

### 同一再現で取得する情報

1. USBを抜く前から再接続後の停止まで、EP0とマイクINを含むusbmonを取得する。デバイスアドレスは010→011のように変わるため、古いアドレスだけでフィルタしない。
2. EP0の`SET_INTERFACE`要求と完了を順に確認する。Alt 0→1とAlt 1→1を区別し、reset由来の通知カウンタだけで順序を推測しない。
3. マイクINのURB submit／completion、status、各ISO descriptorの`actual_length`を確認する。usbmonの`E`はsubmit errorであり、`C`とは別。末尾のバッファ長やゼロ表示だけでPCM受信量を判断しない。[Linux usbmon仕様](https://docs.kernel.org/usb/usbmon.html)
4. デバイス側には次の読出し専用情報を追加する。UARTの長いログをリアルタイム経路へ戻さず、既存のEP0診断と同じ方式で取得する。

| 追加観測 | 確認すること |
|---|---|
| write開始回数・現在の待機段階 | `wait_enabled`、前回転送待ち、FIFO待ちのどこか |
| RAMの`in_enabled` | 診断の`alt`とドライバ状態の一致 |
| `DIEPCTL1` | USBAEP、EPENA、NAK、フレーム偶奇 |
| `DIEPTSIZ1`・`DTXFSTS1` | 未完了転送の残量とFIFO空き |
| `DIEPINT1`・`DIEPMSK`・`DAINTMSK`・`DIEPEMPMSK` | XFRCの未処理／mask、FIFO待ちの割り込み条件 |
| `GINTSTS`・`GINTMSK`・`DSTS` | incomplete-ISO、suspend、SOF進行 |
| incomplete-ISO回数・abort各待ちの失敗回数 | 再試行と停止処理が実際に成功したか |

### 観測による切り分け

| 観測結果 | 次に追う箇所 |
|---|---|
| 再接続後に録音用URBのsubmitがない | Discord／PipeWireの再接続、ALSAストリーム再開 |
| URBのsubmitが`E`で失敗 | ホスト側endpoint有効化とcontrol要求の成否 |
| ホストが録音要求を継続し、デバイスは`EPENA=1`の前回転送待ちで固定 | abort／incomplete-ISO／再有効化による転送状態の復旧 |
| FIFO待ちのままReset／Alt変更をまたぐ | キャンセル確認漏れと接続世代管理 |
| XFRCがpendingのまま計数されない | IN割り込みのmask・処理経路 |
| 両カウンタは進むがISO実受信長が0、またはPCMが無音 | 転送長、FIFO内容、I²Sデータを別途確認 |

`DSTS`のSOF進行だけでは、ホストがマイクendpointをpollしているとは言えない。usbmonとデバイス状態を同じ再現で対応付ける。

## 修正後の受入条件

後続の修正では、Discordを開いたまま録音→USB再接続→録音を少なくとも10回繰り返す。毎回、再開後10秒以上の録音が取得できることを確認する。

- Alt 1でホストが録音中、`queued`と`xfrc`が概ね1,000回/秒で継続して増加する。切断／Alt 0の停止は正常として扱う。
- usbmonで非ゼロ長のマイクPCMが継続して受信される。mono S16の通常パケットは94／96／98 bytesに対応する。
- WAVの録音時間が進み、声に応じて波形が変化する。ヘッダだけのWAVや無音データだけの連続転送を成功としない。
- `queued−xfrc`の累積差がゼロであることは要求しない。再接続時にキャンセルされた過去の予約と、現在の転送停止を混同しない。
- 完全な電源再投入と、MCUの給電を維持したUSBだけの再接続を区別して確認する。

## 実施済みの確認と限界

- 診断スクリプト、EP0ハンドラ、USBマイク送信処理、OTGのwrite／IRQ／Reset／Alt切替をソースで追跡した。
- `python3 scripts/microphone_usb_stats.py --self-test`：成功。診断形式、差分計算、wrap／再起動判定の自己テストであり、実機転送の検証ではない。
- `cargo test --manifest-path dick_mouse/tests/host_audio/Cargo.toml --locked --offline`：lockfileの更新が必要というエラーで実行できなかった。テスト成功とは扱わず、lockfileも変更していない。
- 実機の停止時レジスタ、今回と同時刻のusbmon、書き込み済みファームウェアと対象コミットの一致は未確認。
- この作業は報告書の追加のみ。転送の修正や再接続成功の実機確認は実施していない。

## 修正実施（2026-09-22）

承認済みの計画に従い、以下を実装した。**コード上の問題候補を修正したものであり、提示ログの具体的な停止原因の確定・実機での解決確認とは区別する。**

- IN endpointの接続世代を追加。Reset／Alt再設定／電源除去／deinitで古いwriteを無効化する。前回転送待ち・FIFO待ち・予約を一つのpoll内の排他区間へまとめ、旧writeが新しい接続に予約されないようにした。
- 同じAlt 1の再設定でも、転送停止確認→FIFO flush→古い割り込みと転送長のクリア→有効化の順にした。従来のFIFOだけを先に消す経路を除去した。
- NAK待ち・EPENA解除待ち・TX FIFO flushの失敗を呼び出し元へ返す。失敗したendpointを再有効化せず、エラー種別を計数する。古いEPDISDだけを今回の停止成功とは扱わない。
- incomplete-ISOは無効endpointやReset等の処理中に再予約せず、abort失敗時も再予約しない。通常の再試行方式自体は変更していない。
- ResetではINの無効化と停止をFIFO flushより先に行う。再接続では確立済みのFIFO配置を維持し、成功したendpointのFIFOだけを個別にflushして再設定する。マイク停止失敗だけでEP0まで無効化しない。失敗したendpointは次のAlt設定でtype／MPS／FIFOを含めて再設定できる。ただし、初回の共有FIFO配置確立前に停止が失敗した場合や、EP0自身の停止／flushが失敗した場合までEP0応答を保証するものではない。
- 新しいEP0診断`MIC2`（112 bytes、index=1）とHAL読出し関数を追加。write開始・キャンセル・世代・待機段階・有効状態・incomplete／timeout回数・関連レジスタを読める。既存`MIC1`（32 bytes、index=0）の形式とqueue／XFRC計数位置は維持した。
- 診断スクリプトはMIC2を優先し、旧ファームウェアのSTALL時だけMIC1へ戻る。レジスタ値は表示するだけで、W1CクリアやFIFO読出しを行わない。

依存リビジョン、マイク／スピーカーdescriptor、I²S・リング・ゲインは変更していない。リアルタイム経路へのUARTログ、USB全体の自動リセット／再列挙も追加していない。詳細なプロトコルと制限は[ローカルパッチ説明](vendor/embassy-usb-synopsys-otg/DXXK_PATCH.md)に記載した。

### 自動確認

- `cargo +esp test --manifest-path vendor/embassy-usb-synopsys-otg/Cargo.toml --locked --offline --lib`：9件成功。
- `cargo +esp test --manifest-path dick_mouse/tests/host_audio/Cargo.toml --locked --offline`：5件成功。ホストテスト用lockfileの不足していたupstream `embassy-usb` エントリを補った。既存の依存バージョンは更新していない。
- `python3 scripts/microphone_usb_stats.py --self-test`：成功。MIC1/MIC2形式、長さ不正、表示bit、fallback、非STALLエラー伝播、カウンタwrap／再起動判定、USBアドレス／reset変更時の差分基準更新を確認した。
- `dick_mouse`ディレクトリで`cargo +esp build --release --locked --offline --bin dick_mouse`：成功。
- 同じrelease buildで`microphone_ring_buffer`と`speaker_ring_buffer`のexampleも確認した。
- ホストテストを通常のstable 1.94で実行すると、固定依存xarxaの`cfg_select!`が未対応のため失敗する。ファームウェアと同じ`+esp`では成功する。依存更新で回避していない。

OTGテストは実際のwrite future／IRQ／停止関数をRAM上のレジスタで実行する。ハードウェアによるW1CやEPENA／flush bitの自動変化はエミュレートしていないため、正常なNAK／EPDIS完了やUSBフレームタイミングの実機確認の代用にはならない。

### 次の実機確認

この作業では書き込み・ケーブル再接続を行っていない。ビルド済みファームウェアを書き込んだ後、以下を録音開始前から取得する。USBアドレスが変わるため`--address`は固定しない。

```sh
sudo python3 scripts/microphone_usb_stats.py
```

Discordを開いたまま再接続テストを10回行い、各回10秒以上の録音と、同じ再現のusbmonを保存する。スピーカーとHIDも確認する。

- `phase=wait_previous`かつ`EPENA=1`で固定：ホストの録音URB継続有無と、incomplete／timeoutの増分を照合する。
- `phase=wait_fifo`で固定：`DTXFSTS`／`DIEPEMPMSK`を確認する。世代変更をまたいだ旧writeはキャンセルされるべきである。
- `enabled=0`かつtimeoutが増加：停止処理が実際に失敗した。`last_error`は過去の失敗も保持するため、差分と併せて判断する。
- queue／XFRCが進んでも、非ゼロ長PCMの受信・録音時間・声に応じた波形が確認できなければ成功としない。

自動リセットで状態を隠さず、追加診断でホスト未再開とハードウェアの停止失敗を切り分ける。

## MIC2実測後の追加修正（2026-09-22）

### 今回確定した停止箇所

22:18:09〜22:19:18の提示ログでは、アドレス022、023、024で同じ状態に停止している。

```text
alt=1 phase=wait_fifo enabled=1
USBAEP=1 EPENA=0 DIEPTSIZ=0x00000000 remaining=0
fifo_words=5 DTXFSTS=0x00000005 DIEPEMPMSK=0x00000002
XFRC=0 timeouts=0/0/0 last_error=none
```

- `wait_previous`ではなく**次のパケットのFIFO空き待ち**。今回の停止中に「EPENAが1のまま残った」「abortがtimeoutした」という仮説は当てはまらない。
- `fifo_words`は容量ではなく空きワード数。94／96 bytesには24ワード、98 bytesには25ワードが必要だが、5ワード（20 bytes）しか空いていない。コード上のマイクTX FIFO割当は25ワード。これはソフトウェアの音声リング不足ではない。
- 転送有効bitもPKTCNT／XFRSIZも0で、新たなデータを送る転送が予約されていない。FIFO不足を解消する転送がないまま、writeがTXFEを待つ循環待ちになっている。
- `DIEPINT=0x2010`ではXFRC（bit 0）もTXFE（bit 7）も0。FIFO-empty割り込みのmaskを有効にするだけでは、この停止状態から進まない。
- 22:18:25にはAlt 0／1と世代・キャンセルが増え、queued／XFRCも各1増えた後に再び同じ停止になる。旧writeのキャンセルは働いているが、次の転送サイクルで再発している。
- アドレス変更後の023でも、累積カウンタが小さくなった024でも再発した。Discordの古いデバイス選択だけでは、このデバイス内FIFO待ちを説明できない。ただし、同時刻のホストURB状況は依然未取得。

**確定：転送のないISO INでFIFO空きを永久待機するソフトウェア経路。最有力の発生経路：incomplete-ISO後にFIFOを保持し、転送サイズを再構築せずに再有効化する処理。**

停止前に`incomplete=26`、Alt切替後30、再接続後52となっており、この処理は実際に通っている。ただし、残留データが生まれた瞬間のTSIZ／FIFOを連続採取したわけではないため、残ったワードの内容や各ハードウェア更新の順序まで断定しない。

参考として[TinyUSBのDWC2実装](https://github.com/hathach/tinyusb/blob/master/src/portable/synopsys/dwc2/dcd_dwc2.c)では、incomplete-ISO再試行時に転送サイズを再設定し、再試行を断念する際にはendpoint停止・TX FIFO flushを行う。今回の実装はそのコードの移植ではなく、既存の停止／flush関数を再利用して期限切れパケットから次へ進めるもの。

### 追加した修正

1. incomplete-ISOでは、停止成功を確認して対象のTX FIFOをflushし、古い割り込み・転送サイズをクリアする。古いFIFOをそのまま再有効化する処理を削除した。
2. 成功した場合はendpointのソフトウェア有効状態と接続世代を維持し、待機中の次のwriteをwakeする。次のwriteが新しいPCM・転送長・フレーム偶奇を設定する。期限切れパケットをXFRC成功として計数しない。
3. write側でも、ISO INかつ`EPENA=0`／`PKTCNT=0`／`XFRSIZ=0`でFIFO不足の場合に限り、そのFIFOを整理して空きを再確認する。今回実測した待機状態そのものから回復する経路であり、稼働中の転送やControl／Bulk／Interrupt INはflushしない。
4. 停止／flush失敗は既存カウンタへ記録し、`Disabled`を返す。成功扱いで転送を強行したり、USB全体をリセットしたりしない。

ISO IN共通の処理なのでスピーカーのfeedback INにも適用される。音声OUT、I²S DMA、descriptor、リング、ゲイン、依存バージョンは今回変更していない。MIC1／MIC2の形式と読取スクリプトも維持した。期限切れパケットの破棄は異常時の復旧であり、通常のサンプルレート補正として使うものではない。

### 検証と限界

- OTGドライバ：13テスト成功。今回の実測値（空き5ワード、EPENA／TSIZ=0）から94／96／98 bytesを再予約できること、期限切れ処理が古い転送を再有効化せず次のwriteをwakeすること、両方のフレーム偶奇、flush失敗、稼働中／非ISO転送の保護を追加確認した。
- 音声側ホストテスト：5件成功。診断スクリプトの`--self-test`も成功。
- `cargo +esp build --release --locked --offline --bin dick_mouse --example microphone_ring_buffer --example speaker_ring_buffer`：成功（`dick_mouse`ディレクトリで実行）。
- 正常系のテストには最小限のNAK／EPDIS／flushモデルを追加した。モデル上でabort後のTSIZを消し、それに依存した再送をしないことを確認する。実機の全レジスタ動作やフレームタイミングを再現したものではない。

実機での書き込み・録音・USB抜き差しは未実施。次のビルドを書き込んで同じ診断スクリプトを実行し、録音中に`dqueued`／`dxfrc`がともに約1,000/秒へ戻るか確認する。再接続10回、各回10秒以上の録音、スピーカーとHIDの同時動作も確認する。

`incomplete`が録音中に毎秒多数増える、queuedだけが増えてXFRCが増えない、または非ゼロ長のPCMを受信できない場合は解決とはしない。usbmonのISO descriptor実受信長と実音声を併せて確認する。flush後も空き5ワードに固定する場合は、次に実際のTX FIFO配置レジスタとFIFO使用状態を調べる必要がある。
