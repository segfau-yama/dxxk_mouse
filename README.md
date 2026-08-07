# dxxk_mouse仕様
esp-hal<=>adapter
# dxxk_mouseテスト項目
## button.rs
1. stateがtrueの場合、is_high()でtrueをreturnする
1. stateがfalseの場合、is_high()でfalseをreturnする
1. stateがfalseの場合、is_low()でtrueをreturnする
1. stateがtrueの場合、is_low()でfalseをreturnする
1. past_stateとstateが異なった場合、is_chatteringをtrueにする

## encoder.rs
1. 