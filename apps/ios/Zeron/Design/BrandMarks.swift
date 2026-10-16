// Harness brand marks — the exact SVG path data the desktop bundles
// (crates/ui/assets/icons/{claude,openai,cursor}-mark.svg), rendered natively
// via a small SVG path-data parser → SwiftUI Path. Marks with a brand color
// (`BrandMark.color`) keep it everywhere; black-or-white marks tint with the
// foreground style like any glyph.

import SwiftUI
import UIKit

enum BrandMark {
    case claude, openai, cursor, devin, grok, hermes, pi, opencode, antigravity
    // Model vendors (Lobe Icons, MIT): grouping in the model picker.
    case gemini, deepseek, qwen, kimi, zai, minimax, mistral, nvidia, meta, xiaomi, longcat

    var viewBox: CGSize {
        switch self {
        case .claude: return CGSize(width: 256, height: 257)
        case .openai: return CGSize(width: 256, height: 260)
        case .cursor: return CGSize(width: 466.73, height: 532.09)
        case .devin: return CGSize(width: 263, height: 300)
        case .grok: return CGSize(width: 16, height: 16)
        case .hermes: return CGSize(width: 24, height: 24)
        case .pi: return CGSize(width: 800, height: 800)
        case .opencode: return CGSize(width: 24, height: 30)
        case .antigravity, .gemini, .deepseek, .qwen, .kimi, .zai, .minimax, .mistral, .nvidia, .meta, .xiaomi, .longcat: return CGSize(width: 24, height: 24)
        }
    }

    /// The source SVG's fill-rule; the shape must fill even-odd where the
    /// asset says so or the mark's holes fill in solid.
    var evenOddFill: Bool {
        switch self {
        case .hermes, .pi, .opencode, .antigravity, .gemini, .deepseek, .qwen, .kimi, .zai, .minimax, .mistral, .nvidia, .meta, .xiaomi, .longcat: return true
        default: return false
        }
    }

    var pathData: String {
        switch self {
        case .claude:
            return "m50.228 170.321 50.357-28.257.843-2.463-.843-1.361h-2.462l-8.426-.518-28.775-.778-24.952-1.037-24.175-1.296-6.092-1.297L0 125.796l.583-3.759 5.12-3.434 7.324.648 16.202 1.101 24.304 1.685 17.629 1.037 26.118 2.722h4.148l.583-1.685-1.426-1.037-1.101-1.037-25.147-17.045-27.22-18.017-14.258-10.37-7.713-5.25-3.888-4.925-1.685-10.758 7-7.713 9.397.649 2.398.648 9.527 7.323 20.35 15.75L94.817 91.9l3.889 3.24 1.555-1.102.195-.777-1.75-2.917-14.453-26.118-15.425-26.572-6.87-11.018-1.814-6.61c-.648-2.723-1.102-4.991-1.102-7.778l7.972-10.823L71.42 0 82.05 1.426l4.472 3.888 6.61 15.101 10.694 23.786 16.591 32.34 4.861 9.592 2.592 8.879.973 2.722h1.685v-1.556l1.36-18.211 2.528-22.36 2.463-28.776.843-8.1 4.018-9.722 7.971-5.25 6.222 2.981 5.12 7.324-.713 4.73-3.046 19.768-5.962 30.98-3.889 20.739h2.268l2.593-2.593 10.499-13.934 17.628-22.036 7.778-8.749 9.073-9.657 5.833-4.601h11.018l8.1 12.055-3.628 12.443-11.342 14.388-9.398 12.184-13.48 18.147-8.426 14.518.778 1.166 2.01-.194 30.46-6.481 16.462-2.982 19.637-3.37 8.88 4.148.971 4.213-3.5 8.62-20.998 5.184-24.628 4.926-36.682 8.685-.454.324.519.648 16.526 1.555 7.065.389h17.304l32.21 2.398 8.426 5.574 5.055 6.805-.843 5.184-12.962 6.611-17.498-4.148-40.83-9.721-14-3.5h-1.944v1.167l11.666 11.406 21.387 19.314 26.767 24.887 1.36 6.157-3.434 4.86-3.63-.518-23.526-17.693-9.073-7.972-20.545-17.304h-1.36v1.814l4.73 6.935 25.017 37.59 1.296 11.536-1.814 3.76-6.481 2.268-7.13-1.297-14.647-20.544-15.1-23.138-12.185-20.739-1.49.843-7.194 77.448-3.37 3.953-7.778 2.981-6.48-4.925-3.436-7.972 3.435-15.749 4.148-20.544 3.37-16.333 3.046-20.285 1.815-6.74-.13-.454-1.49.194-15.295 20.999-23.267 31.433-18.406 19.702-4.407 1.75-7.648-3.954.713-7.064 4.277-6.286 25.47-32.405 15.36-20.092 9.917-11.6-.065-1.686h-.583L44.07 198.125l-12.055 1.555-5.185-4.86.648-7.972 2.463-2.593 20.35-13.999-.064.065Z"
        case .openai:
            return "M239.184 106.203a64.716 64.716 0 0 0-5.576-53.103C219.452 28.459 191 15.784 163.213 21.74A65.586 65.586 0 0 0 52.096 45.22a64.716 64.716 0 0 0-43.23 31.36c-14.31 24.602-11.061 55.634 8.033 76.74a64.665 64.665 0 0 0 5.525 53.102c14.174 24.65 42.644 37.324 70.446 31.36a64.72 64.72 0 0 0 48.754 21.744c28.481.025 53.714-18.361 62.414-45.481a64.767 64.767 0 0 0 43.229-31.36c14.137-24.558 10.875-55.423-8.083-76.483Zm-97.56 136.338a48.397 48.397 0 0 1-31.105-11.255l1.535-.87 51.67-29.825a8.595 8.595 0 0 0 4.247-7.367v-72.85l21.845 12.636c.218.111.37.32.409.563v60.367c-.056 26.818-21.783 48.545-48.601 48.601Zm-104.466-44.61a48.345 48.345 0 0 1-5.781-32.589l1.534.921 51.722 29.826a8.339 8.339 0 0 0 8.441 0l63.181-36.425v25.221a.87.87 0 0 1-.358.665l-52.335 30.184c-23.257 13.398-52.97 5.431-66.404-17.803ZM23.549 85.38a48.499 48.499 0 0 1 25.58-21.333v61.39a8.288 8.288 0 0 0 4.195 7.316l62.874 36.272-21.845 12.636a.819.819 0 0 1-.767 0L41.353 151.53c-23.211-13.454-31.171-43.144-17.804-66.405v.256Zm179.466 41.695-63.08-36.63L161.73 77.86a.819.819 0 0 1 .768 0l52.233 30.184a48.6 48.6 0 0 1-7.316 87.635v-61.391a8.544 8.544 0 0 0-4.4-7.213Zm21.742-32.69-1.535-.922-51.619-30.081a8.39 8.39 0 0 0-8.492 0L99.98 99.808V74.587a.716.716 0 0 1 .307-.665l52.233-30.133a48.652 48.652 0 0 1 72.236 50.391v.205ZM88.061 139.097l-21.845-12.585a.87.87 0 0 1-.41-.614V65.685a48.652 48.652 0 0 1 79.757-37.346l-1.535.87-51.67 29.825a8.595 8.595 0 0 0-4.246 7.367l-.051 72.697Zm11.868-25.58 28.138-16.217 28.188 16.218v32.434l-28.086 16.218-28.188-16.218-.052-32.434Z"
        case .cursor:
            return "M457.43,125.94L244.42,2.96c-6.84-3.95-15.28-3.95-22.12,0L9.3,125.94c-5.75,3.32-9.3,9.46-9.3,16.11v247.99c0,6.65,3.55,12.79,9.3,16.11l213.01,122.98c6.84,3.95,15.28,3.95,22.12,0l213.01-122.98c5.75-3.32,9.3-9.46,9.3-16.11v-247.99c0-6.65-3.55-12.79-9.3-16.11h-.01ZM444.05,151.99l-205.63,356.16c-1.39,2.4-5.06,1.42-5.06-1.36v-233.21c0-4.66-2.49-8.97-6.53-11.31L24.87,145.67c-2.4-1.39-1.42-5.06,1.36-5.06h411.26c5.84,0,9.49,6.33,6.57,11.39h-.01Z"
        case .devin:
            return "M1.84577e-10 100.797V38.6224C1.88813e-05 35.8899 1.45777 33.365 3.82414 31.9988L57.6539 0.920185C60.0203 -0.446051 62.9358 -0.446051 65.3022 0.920185L119.132 31.9988C121.498 33.3651 122.956 35.8899 122.956 38.6224V70.375C123.192 80.9262 128.769 91.103 138.577 96.7655C148.384 102.428 159.986 102.169 169.242 97.0979L196.74 81.2217C199.107 79.8554 202.022 79.8554 204.388 81.2217L258.218 112.3C260.585 113.667 262.042 116.191 262.042 118.924V181.081C262.042 183.814 260.585 186.339 258.218 187.705L204.388 218.783C202.022 220.15 199.107 220.15 196.74 218.783L169.464 203.036C160.173 197.847 148.465 197.536 138.579 203.244C128.772 208.906 123.195 219.083 122.959 229.634V261.378C122.959 264.111 121.501 266.635 119.135 268.002L65.3048 299.08C62.9384 300.447 60.0229 300.447 57.6566 299.08L3.82673 268.002C1.46036 266.635 0.00260562 264.111 0.00260562 261.378V199.221C0.0026245 196.488 1.46038 193.963 3.82675 192.597L57.6565 161.519C60.0229 160.152 62.9384 160.152 65.3048 161.519L92.8656 177.431C102.11 182.467 113.679 182.711 123.463 177.062C133.349 171.355 138.934 161.06 139.086 150.419C138.85 139.868 133.274 128.601 123.466 122.938C113.658 117.276 102.056 117.534 92.8007 122.606L65.1758 138.645C62.7991 140.025 59.8649 140.024 57.4894 138.642L3.80216 107.407C1.44816 106.038 -1.88809e-05 103.52 1.84577e-10 100.797Z"
        case .grok:
            return "M0.58392448,14.9254204 L0.8326,14.66295 C2.004465,13.42985 3.1678,12.2082 2.45814,10.48105 C1.50813,8.1701 2.061355,5.4619 3.820575,3.70057 C5.6495,1.8709 8.3431,1.40957 10.59295,2.336505 C11.0907,2.52161 11.5245,2.785025 11.86295,3.02993 L9.98425,3.89849 C8.235,3.163775 6.23115,3.66355 5.0081,4.88809 C3.354105,6.5426 3.019895,9.4117 4.95835,11.2656 L-0.335,15.99995 C-0.066496,15.62975 0.2538896,15.275934 0.58392448,14.9254204 Z M14.0391,2.288155 L16.33165,0 L16.20795,0.172288 C14.4658,2.574355 13.6153,3.749045 14.29795,6.6879 C14.76445,8.68415 14.261,10.90255 12.63545,12.53005 C10.5861,14.58325 7.3066,15.0403 4.6059,13.19215 L6.48885,12.3193 C8.2125,12.99705 10.0983,12.69945 11.4536,11.34255 C12.80895,9.9856 13.1133,8.00925 12.4321,6.3647 C12.30265,6.05285 11.9144,5.97455 11.64275,6.1753 L6.102,10.27035 L14.0391,2.288155 Z"
        case .hermes:
            return HermesMarkData.pathData
        case .pi:
            return "M165.29 165.29H517.36V400H400V517.36H282.65V634.72H165.29ZM282.65 282.65V400H400V282.65ZM517.36 400H634.72V634.72H517.36Z"
        case .opencode:
            // opencode-mark.svg's frame (the desktop asset's second path is a
            // 45%-opacity inner fill this single-path renderer skips — the
            // mark reads identically at glyph sizes).
            return "M24 0H0V30H24V0ZM18 6H6V24H18V6Z"
        case .antigravity:
            return "M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z"
        case .gemini:
            return "M 20.616 10.835 a 14.147 14.147 0 0 1 -4.45 -3.001 14.111 14.111 0 0 1 -3.678 -6.452 .503 .503 0 0 0 -.975 0 14.134 14.134 0 0 1 -3.679 6.452 14.155 14.155 0 0 1 -4.45 3.001 c -.65 .28 -1.318 .505 -2.002 .678 a .502 .502 0 0 0 0 .975 c .684 .172 1.35 .397 2.002 .677 a 14.147 14.147 0 0 1 4.45 3.001 14.112 14.112 0 0 1 3.679 6.453 .502 .502 0 0 0 .975 0 c .172 -.685 .397 -1.351 .677 -2.003 a 14.145 14.145 0 0 1 3.001 -4.45 14.113 14.113 0 0 1 6.453 -3.678 .503 .503 0 0 0 0 -.975 13.245 13.245 0 0 1 -2.003 -.678 z"
        case .deepseek:
            return "M 23.748 4.482 c -.254 -.124 -.364 .113 -.512 .234 -.051 .039 -.094 .09 -.137 .136 -.372 .397 -.806 .657 -1.373 .626 -.829 -.046 -1.537 .214 -2.163 .848 -.133 -.782 -.575 -1.248 -1.247 -1.548 -.352 -.156 -.708 -.311 -.955 -.65 -.172 -.241 -.219 -.51 -.305 -.774 -.055 -.16 -.11 -.323 -.293 -.35 -.2 -.031 -.278 .136 -.356 .276 -.313 .572 -.434 1.202 -.422 1.84 .027 1.436 .633 2.58 1.838 3.393 .137 .093 .172 .187 .129 .323 -.082 .28 -.18 .552 -.266 .833 -.055 .179 -.137 .217 -.329 .14 a 5.526 5.526 0 0 1 -1.736 -1.18 c -.857 -.828 -1.631 -1.742 -2.597 -2.458 a 11.365 11.365 0 0 0 -.689 -.471 c -.985 -.957 .13 -1.743 .388 -1.836 .27 -.098 .093 -.432 -.779 -.428 -.872 .004 -1.67 .295 -2.687 .684 a 3.055 3.055 0 0 1 -.465 .137 9.597 9.597 0 0 0 -2.883 -.102 c -1.885 .21 -3.39 1.102 -4.497 2.623 C .082 8.606 -.231 10.684 .152 12.85 c .403 2.284 1.569 4.175 3.36 5.653 1.858 1.533 3.997 2.284 6.438 2.14 1.482 -.085 3.133 -.284 4.994 -1.86 .47 .234 .962 .327 1.78 .397 .63 .059 1.236 -.03 1.705 -.128 .735 -.156 .684 -.837 .419 -.961 -2.155 -1.004 -1.682 -.595 -2.113 -.926 1.096 -1.296 2.746 -2.642 3.392 -7.003 .05 -.347 .007 -.565 0 -.845 -.004 -.17 .035 -.237 .23 -.256 a 4.173 4.173 0 0 0 1.545 -.475 c 1.396 -.763 1.96 -2.015 2.093 -3.517 .02 -.23 -.004 -.467 -.247 -.588 z M 11.581 18 c -2.089 -1.642 -3.102 -2.183 -3.52 -2.16 -.392 .024 -.321 .471 -.235 .763 .09 .288 .207 .486 .371 .739 .114 .167 .192 .416 -.113 .603 -.673 .416 -1.842 -.14 -1.897 -.167 -1.361 -.802 -2.5 -1.86 -3.301 -3.307 -.774 -1.393 -1.224 -2.887 -1.298 -4.482 -.02 -.386 .093 -.522 .477 -.592 a 4.696 4.696 0 0 1 1.529 -.039 c 2.132 .312 3.946 1.265 5.468 2.774 .868 .86 1.525 1.887 2.202 2.891 .72 1.066 1.494 2.082 2.48 2.914 .348 .292 .625 .514 .891 .677 -.802 .09 -2.14 .11 -3.054 -.614 z m 1 -6.44 a .306 .306 0 0 1 .415 -.287 .302 .302 0 0 1 .2 .288 .306 .306 0 0 1 -.31 .307 .303 .303 0 0 1 -.304 -.308 z m 3.11 1.596 c -.2 .081 -.399 .151 -.59 .16 a 1.245 1.245 0 0 1 -.798 -.254 c -.274 -.23 -.47 -.358 -.552 -.758 a 1.73 1.73 0 0 1 .016 -.588 c .07 -.327 -.008 -.537 -.239 -.727 -.187 -.156 -.426 -.199 -.688 -.199 a .559 .559 0 0 1 -.254 -.078 c -.11 -.054 -.2 -.19 -.114 -.358 .028 -.054 .16 -.186 .192 -.21 .356 -.202 .767 -.136 1.146 .016 .352 .144 .618 .408 1.001 .782 .391 .451 .462 .576 .685 .914 .176 .265 .336 .537 .445 .848 .067 .195 -.019 .354 -.25 .452 z"
        case .qwen:
            return "M 12.604 1.34 c .393 .69 .784 1.382 1.174 2.075 a .18 .18 0 0 0 .157 .091 h 5.552 c .174 0 .322 .11 .446 .327 l 1.454 2.57 c .19 .337 .24 .478 .024 .837 -.26 .43 -.513 .864 -.76 1.3 l -.367 .658 c -.106 .196 -.223 .28 -.04 .512 l 2.652 4.637 c .172 .301 .111 .494 -.043 .77 -.437 .785 -.882 1.564 -1.335 2.34 -.159 .272 -.352 .375 -.68 .37 -.777 -.016 -1.552 -.01 -2.327 .016 a .099 .099 0 0 0 -.081 .05 575.097 575.097 0 0 1 -2.705 4.74 c -.169 .293 -.38 .363 -.725 .364 -.997 .003 -2.002 .004 -3.017 .002 a .537 .537 0 0 1 -.465 -.271 l -1.335 -2.323 a .09 .09 0 0 0 -.083 -.049 H 4.982 c -.285 .03 -.553 -.001 -.805 -.092 l -1.603 -2.77 a .543 .543 0 0 1 -.002 -.54 l 1.207 -2.12 a .198 .198 0 0 0 0 -.197 550.951 550.951 0 0 1 -1.875 -3.272 l -.79 -1.395 c -.16 -.31 -.173 -.496 .095 -.965 .465 -.813 .927 -1.625 1.387 -2.436 .132 -.234 .304 -.334 .584 -.335 a 338.3 338.3 0 0 1 2.589 -.001 .124 .124 0 0 0 .107 -.063 l 2.806 -4.895 a .488 .488 0 0 1 .422 -.246 c .524 -.001 1.053 0 1.583 -.006 L 11.704 1 c .341 -.003 .724 .032 .9 .34 z m -3.432 .403 a .06 .06 0 0 0 -.052 .03 L 6.254 6.788 a .157 .157 0 0 1 -.135 .078 H 3.253 c -.056 0 -.07 .025 -.041 .074 l 5.81 10.156 c .025 .042 .013 .062 -.034 .063 l -2.795 .015 a .218 .218 0 0 0 -.2 .116 l -1.32 2.31 c -.044 .078 -.021 .118 .068 .118 l 5.716 .008 c .046 0 .08 .02 .104 .061 l 1.403 2.454 c .046 .081 .092 .082 .139 0 l 5.006 -8.76 .783 -1.382 a .055 .055 0 0 1 .096 0 l 1.424 2.53 a .122 .122 0 0 0 .107 .062 l 2.763 -.02 a .04 .04 0 0 0 .035 -.02 .041 .041 0 0 0 0 -.04 l -2.9 -5.086 a .108 .108 0 0 1 0 -.113 l .293 -.507 1.12 -1.977 c .024 -.041 .012 -.062 -.035 -.062 H 9.2 c -.059 0 -.073 -.026 -.043 -.077 l 1.434 -2.505 a .107 .107 0 0 0 0 -.114 L 9.225 1.774 a .06 .06 0 0 0 -.053 -.031 z m 6.29 8.02 c .046 0 .058 .02 .034 .06 l -.832 1.465 -2.613 4.585 a .056 .056 0 0 1 -.05 .029 .058 .058 0 0 1 -.05 -.029 L 8.498 9.841 c -.02 -.034 -.01 -.052 .028 -.054 l .216 -.012 6.722 -.012 z"
        case .kimi:
            return "M 21.846 0 a 1.923 1.923 0 1 1 0 3.846 H 20.15 a .226 .226 0 0 1 -.227 -.226 V 1.923 C 19.923 .861 20.784 0 21.846 0 z M 11.065 11.199 l 7.257 -7.2 c .137 -.136 .06 -.41 -.116 -.41 H 14.3 a .164 .164 0 0 0 -.117 .051 l -7.82 7.756 c -.122 .12 -.302 .013 -.302 -.179 V 3.82 c 0 -.127 -.083 -.23 -.185 -.23 H 3.186 c -.103 0 -.186 .103 -.186 .23 V 19.77 c 0 .128 .083 .23 .186 .23 h 2.69 c .103 0 .186 -.102 .186 -.23 v -3.25 c 0 -.069 .025 -.135 .069 -.178 l 2.424 -2.406 a .158 .158 0 0 1 .205 -.023 l 6.484 4.772 a 7.677 7.677 0 0 0 3.453 1.283 c .108 .012 .2 -.095 .2 -.23 v -3.06 c 0 -.117 -.07 -.212 -.164 -.227 a 5.028 5.028 0 0 1 -2.027 -.807 l -5.613 -4.064 c -.117 -.078 -.132 -.279 -.028 -.381 z"
        case .zai:
            return "M 12.105 2 L 9.927 4.953 H .653 L 2.83 2 h 9.276 z M 23.254 19.048 L 21.078 22 h -9.242 l 2.174 -2.952 h 9.244 z M 24 2 L 9.264 22 H 0 L 14.736 2 H 24 z"
        case .minimax:
            return "M 16.278 2 c 1.156 0 2.093 .927 2.093 2.07 v 12.501 a .74 .74 0 0 0 .744 .709 .74 .74 0 0 0 .743 -.709 V 9.099 a 2.06 2.06 0 0 1 2.071 -2.049 A 2.06 2.06 0 0 1 24 9.1 v 6.561 a .649 .649 0 0 1 -.652 .645 .649 .649 0 0 1 -.653 -.645 V 9.1 a .762 .762 0 0 0 -.766 -.758 .762 .762 0 0 0 -.766 .758 v 7.472 a 2.037 2.037 0 0 1 -2.048 2.026 2.037 2.037 0 0 1 -2.048 -2.026 v -12.5 a .785 .785 0 0 0 -.788 -.753 .785 .785 0 0 0 -.789 .752 l -.001 15.904 A 2.037 2.037 0 0 1 13.441 22 a 2.037 2.037 0 0 1 -2.048 -2.026 V 18.04 c 0 -.356 .292 -.645 .652 -.645 .36 0 .652 .289 .652 .645 v 1.934 c 0 .263 .142 .506 .372 .638 .23 .131 .514 .131 .744 0 a .734 .734 0 0 0 .372 -.638 V 4.07 c 0 -1.143 .937 -2.07 2.093 -2.07 z m -5.674 0 c 1.156 0 2.093 .927 2.093 2.07 v 11.523 a .648 .648 0 0 1 -.652 .645 .648 .648 0 0 1 -.652 -.645 V 4.07 a .785 .785 0 0 0 -.789 -.78 .785 .785 0 0 0 -.789 .78 v 14.013 a 2.06 2.06 0 0 1 -2.07 2.048 2.06 2.06 0 0 1 -2.071 -2.048 V 9.1 a .762 .762 0 0 0 -.766 -.758 .762 .762 0 0 0 -.766 .758 v 3.8 a 2.06 2.06 0 0 1 -2.071 2.049 A 2.06 2.06 0 0 1 0 12.9 v -1.378 c 0 -.357 .292 -.646 .652 -.646 .36 0 .653 .29 .653 .646 V 12.9 c 0 .418 .343 .757 .766 .757 s .766 -.339 .766 -.757 V 9.099 a 2.06 2.06 0 0 1 2.07 -2.048 2.06 2.06 0 0 1 2.071 2.048 v 8.984 c 0 .419 .343 .758 .767 .758 .423 0 .766 -.339 .766 -.758 V 4.07 c 0 -1.143 .937 -2.07 2.093 -2.07 z"
        case .mistral:
            return "M 3.428 3.4 h 3.429 v 3.428 h 3.429 v 3.429 h -.002 3.431 V 6.828 h 3.427 V 3.4 h 3.43 v 13.714 H 24 v 3.429 H 13.714 v -3.428 h -3.428 v -3.429 h -3.43 v 3.428 h 3.43 v 3.429 H 0 v -3.429 h 3.428 V 3.4 z m 10.286 13.715 h 3.428 v -3.429 h -3.427 v 3.429 z"
        case .nvidia:
            return "M 10.212 8.976 V 7.62 c .127 -.01 .256 -.017 .388 -.021 3.596 -.117 5.957 3.184 5.957 3.184 s -2.548 3.647 -5.282 3.647 a 3.227 3.227 0 0 1 -1.063 -.175 v -4.109 c 1.4 .174 1.681 .812 2.523 2.258 l 1.873 -1.627 a 4.905 4.905 0 0 0 -3.67 -1.846 6.594 6.594 0 0 0 -.729 .044 m 0 -4.476 v 2.025 c .13 -.01 .259 -.019 .388 -.024 5.002 -.174 8.261 4.226 8.261 4.226 s -3.743 4.69 -7.643 4.69 c -.338 0 -.675 -.031 -1.007 -.092 v 1.25 c .278 .038 .558 .057 .838 .057 3.629 0 6.253 -1.91 8.794 -4.169 .421 .347 2.146 1.193 2.501 1.564 -2.416 2.083 -8.048 3.763 -11.24 3.763 -.308 0 -.603 -.02 -.894 -.048 V 19.5 H 24 v -15 H 10.21 z m 0 9.756 v 1.068 c -3.356 -.616 -4.287 -4.21 -4.287 -4.21 a 7.173 7.173 0 0 1 4.287 -2.138 v 1.172 h -.005 a 3.182 3.182 0 0 0 -2.502 1.178 s .615 2.276 2.507 2.931 m -5.961 -3.3 c 1.436 -1.935 3.604 -3.148 5.961 -3.336 V 6.523 C 5.81 6.887 2 10.723 2 10.723 s 2.158 6.427 8.21 7.015 v -1.166 C 5.77 16 4.25 10.958 4.25 10.958 h -.002 z"
        case .meta:
            return "M 6.897 4 c 1.915 0 3.516 .932 5.43 3.376 l .282 -.373 c .19 -.246 .383 -.484 .58 -.71 l .313 -.35 C 14.588 4.788 15.792 4 17.225 4 c 1.273 0 2.469 .557 3.491 1.516 l .218 .213 c 1.73 1.765 2.917 4.71 3.053 8.026 l .011 .392 .002 .25 c 0 1.501 -.28 2.759 -.818 3.7 l -.14 .23 -.108 .153 c -.301 .42 -.664 .758 -1.086 1.009 l -.265 .142 -.087 .04 a 3.493 3.493 0 0 1 -.302 .118 4.117 4.117 0 0 1 -1.33 .208 c -.524 0 -.996 -.067 -1.438 -.215 -.614 -.204 -1.163 -.56 -1.726 -1.116 l -.227 -.235 c -.753 -.812 -1.534 -1.976 -2.493 -3.586 l -1.43 -2.41 -.544 -.895 -1.766 3.13 -.343 .592 C 7.597 19.156 6.227 20 4.356 20 c -1.21 0 -2.205 -.42 -2.936 -1.182 l -.168 -.184 c -.484 -.573 -.837 -1.311 -1.043 -2.189 l -.067 -.32 a 8.69 8.69 0 0 1 -.136 -1.288 L 0 14.468 c .002 -.745 .06 -1.49 .174 -2.23 l .1 -.573 c .298 -1.53 .828 -2.958 1.536 -4.157 l .209 -.34 c 1.177 -1.83 2.789 -3.053 4.615 -3.16 L 6.897 4 z m -.033 2.615 l -.201 .01 c -.83 .083 -1.606 .673 -2.252 1.577 l -.138 .199 -.01 .018 c -.67 1.017 -1.185 2.378 -1.456 3.845 l -.004 .022 a 12.591 12.591 0 0 0 -.207 2.254 l .002 .188 c .004 .18 .017 .36 .04 .54 l .043 .291 c .092 .503 .257 .908 .486 1.208 l .117 .137 c .303 .323 .698 .492 1.17 .492 1.1 0 1.796 -.676 3.696 -3.641 l 2.175 -3.4 .454 -.701 -.139 -.198 C 9.11 7.3 8.084 6.616 6.864 6.616 z m 10.196 -.552 l -.176 .007 c -.635 .048 -1.223 .359 -1.82 .933 l -.196 .198 c -.439 .462 -.887 1.064 -1.367 1.807 l .266 .398 c .18 .274 .362 .56 .55 .858 l .293 .475 1.396 2.335 .695 1.114 c .583 .926 1.03 1.6 1.408 2.082 l .213 .262 c .282 .326 .529 .54 .777 .673 l .102 .05 c .227 .1 .457 .138 .718 .138 .176 .002 .35 -.023 .518 -.073 .338 -.104 .61 -.32 .813 -.637 l .095 -.163 .077 -.162 c .194 -.459 .29 -1.06 .29 -1.785 l -.006 -.449 c -.08 -2.871 -.938 -5.372 -2.2 -6.798 l -.176 -.189 c -.67 -.683 -1.444 -1.074 -2.27 -1.074 z"
        case .xiaomi:
            return "M .958 15.936 a .459 .459 0 0 1 .459 .44 v 2.729 a .46 .46 0 0 1 -.918 0 v -2.729 a .459 .459 0 0 1 .459 -.44 z m 4.814 -2.035 a .46 .46 0 0 1 .553 .45 v 4.754 a .458 .458 0 1 1 -.918 0 V 15.48 L 3.74 17.202 a .462 .462 0 0 1 -.655 .016 .462 .462 0 0 1 -.065 -.082 L .628 14.67 a .459 .459 0 0 1 .658 -.637 l 2.124 2.187 2.127 -2.188 a .46 .46 0 0 1 .235 -.13 z m 2.068 .004 a .46 .46 0 0 1 .458 .445 v 4.755 a .46 .46 0 0 1 -.458 .458 .459 .459 0 0 1 -.458 -.458 V 14.35 a .459 .459 0 0 1 .458 -.445 z m 1.973 2.014 a .46 .46 0 0 1 .46 .457 v 2.729 a .46 .46 0 0 1 -.784 .324 .46 .46 0 0 1 -.134 -.324 v -2.729 a .46 .46 0 0 1 .458 -.458 z m .002 -2.045 a .458 .458 0 0 1 .328 .157 l 2.127 2.19 2.125 -2.19 a .459 .459 0 0 1 .784 .318 v 4.756 a .46 .46 0 0 1 -.455 .458 .46 .46 0 0 1 -.458 -.458 V 15.48 l -1.667 1.723 a .46 .46 0 0 1 -.65 .008 l -.005 -.005 c 0 -.002 -.002 -.002 -.004 -.003 l -2.455 -2.534 a .46 .46 0 0 1 -.008 -.667 .461 .461 0 0 1 .338 -.128 z m 6.797 1.206 a .46 .46 0 0 1 .53 .651 A 1.966 1.966 0 0 0 19.81 18.4 a .462 .462 0 0 1 .623 .18 .46 .46 0 0 1 -.181 .624 2.863 2.863 0 0 1 -1.38 .353 l -.142 -.004 a 2.88 2.88 0 0 1 -2.393 -4.263 .461 .461 0 0 1 .274 -.21 z m .864 -.931 a 2.884 2.884 0 0 1 3.915 3.914 .46 .46 0 0 1 -.402 .24 l -.057 -.004 a .458 .458 0 0 1 -.164 -.055 .46 .46 0 0 1 -.182 -.622 1.967 1.967 0 0 0 -2.669 -2.67 .459 .459 0 1 1 -.441 -.803 z M 9.59 6.368 c 1.481 0 1.696 1.202 1.696 1.654 v 2.648 h -.917 v -.432 c -.26 .346 -.792 .535 -1.36 .535 -.133 0 -1.289 -.03 -1.384 -1.136 -.082 -.932 .675 -1.61 2.053 -1.61 h .691 c 0 -.563 -.367 -.886 -.983 -.886 -.44 .013 -.864 .174 -1.2 .458 l -.36 -.664 c .484 -.379 1.012 -.567 1.764 -.567 z m 4.427 .1 c 1.263 0 2.082 .97 2.083 2.15 0 1.181 -.824 2.154 -2.083 2.154 -1.26 0 -2.084 -.972 -2.084 -2.152 0 -1.18 .82 -2.153 2.084 -2.153 z m 6.801 .015 c .68 0 1.202 .465 1.197 1.548 v 2.642 H 21.1 V 8.29 c 0 -.312 -.002 -.98 -.63 -.98 s -.628 .667 -.628 .838 v 2.524 h -.89 V 8.148 c 0 -.17 -.001 -.838 -.63 -.838 -.628 0 -.628 .668 -.628 .98 v 2.383 h -.917 v -4.03 h .917 V 7 a 1.22 1.22 0 0 1 .947 -.516 c .398 0 .76 .193 .982 .686 a 1.321 1.321 0 0 1 1.195 -.686 z m -18.093 .872 l 1.457 -1.772 H 5.32 L 3.311 8.07 l 2.14 2.602 H 4.24 L 2.725 8.796 1.21 10.672 H 0 L 2.138 8.07 .13 5.583 h 1.138 l 1.458 1.772 z m 4.149 3.317 h -.916 V 6.644 h .916 v 4.028 z m 16.99 0 h -.916 V 6.644 h .916 v 4.028 z M 9.925 8.71 c -1.055 0 -1.359 .412 -1.326 .742 .032 .329 .324 .537 .757 .537 a 1.013 1.013 0 0 0 1.014 -.968 l .002 -.31 h -.447 z M 14.018 7.3 c -.663 0 -1.184 .487 -1.184 1.32 0 .832 .52 1.32 1.184 1.32 .662 0 1.182 -.49 1.182 -1.32 0 -.832 -.52 -1.32 -1.182 -1.32 z M 6.417 5.001 a .568 .568 0 0 1 .587 .582 .588 .588 0 0 1 -1.175 0 A .57 .57 0 0 1 6.417 5 z m 16.991 0 a .57 .57 0 0 1 .592 .582 .588 .588 0 0 1 -1.174 0 .57 .57 0 0 1 .357 -.542 .572 .572 0 0 1 .225 -.04 z"
        case .longcat:
            return "M .507 19.883 a .507 .507 0 0 1 -.489 -.642 L 4.29 3.745 a 1.013 1.013 0 0 1 1.533 -.578 l 5.622 3.687 a 1.013 1.013 0 0 0 1.11 0 L 18.2 3.165 a 1.013 1.013 0 0 1 1.532 .58 l 4.25 15.497 a .506 .506 0 0 1 -.49 .64 H 18.07 a 6.297 6.297 0 0 0 1.53 -4.115 v -.177 a 6.09 6.09 0 0 0 -1.513 -4.017 l -.697 -3.495 a .438 .438 0 0 0 -.694 -.266 L 14.07 9.781 a .748 .748 0 0 1 -.654 .121 5.156 5.156 0 0 0 -2.833 0 .746 .746 0 0 1 -.653 -.121 L 7.302 7.81 a .435 .435 0 0 0 -.688 .269 l -.675 3.652 a 5.36 5.36 0 0 0 -1.539 3.76 v .333 c 0 1.474 .527 2.9 1.488 4.02 l .032 .038 H .507 z M 9.213 16.843 h 1.52 v -3.546 h -1.29 l -.23 3.546 z m 5.573 0 h -1.52 v -3.546 h 1.29 l .23 3.546 z"
        }
    }

    static func forHarness(_ harness: String) -> BrandMark {
        switch harness {
        case "codex": return .openai
        case "cursor": return .cursor
        case "devin": return .devin
        case "grok": return .grok
        case "hermes": return .hermes
        case "pi": return .pi
        case "opencode": return .opencode
        case "antigravity": return .antigravity
        default: return .claude  // claude-code + mock share the mark, like the desktop
        }
    }

    /// The brand's own color; nil for black-or-white marks, which are
    /// templates tinted by the caller.
    var color: UIColor? {
        switch self {
        case .claude: return UIColor(hex: 0xD97757)
        case .devin: return UIColor(hex: 0x0294DE)
        case .antigravity: return UIColor(hex: 0x3186FF)
        case .gemini: return UIColor(hex: 0x3186FF)
        case .deepseek: return UIColor(hex: 0x4D6BFE)
        case .qwen: return UIColor(hex: 0x6F69F7)
        case .minimax: return UIColor(hex: 0xF0365E)
        case .mistral: return UIColor(hex: 0xFA500F)
        case .nvidia: return UIColor(hex: 0x76B900)
        case .meta: return UIColor(hex: 0x0081FB)
        case .longcat: return UIColor(hex: 0x29E154)
        default: return nil
        }
    }
}

/// A `Shape` that renders SVG path data scaled to fit its rect (aspect-fit,
/// centered) — the SwiftUI analogue of gpui's tinted `svg()` element.
struct BrandMarkShape: Shape {
    let mark: BrandMark

    func path(in rect: CGRect) -> Path {
        let base = SVGPathParser.path(from: mark.pathData)
        let box = mark.viewBox
        let scale = min(rect.width / box.width, rect.height / box.height)
        let dx = rect.minX + (rect.width - box.width * scale) / 2
        let dy = rect.minY + (rect.height - box.height * scale) / 2
        return base.applying(CGAffineTransform(scaleX: scale, y: scale)
            .concatenating(CGAffineTransform(translationX: dx, y: dy)))
    }
}

// MARK: - SVG path-data parser

/// Minimal SVG 1.1 path grammar: M/L/H/V/C/S/Q/T/A/Z, absolute + relative.
/// Arcs convert via the standard endpoint→center parameterization.
enum SVGPathParser {
    static func path(from data: String) -> Path {
        var path = Path()
        var scanner = Tokenizer(data)
        var current = CGPoint.zero
        var start = CGPoint.zero
        var lastCubicControl: CGPoint?
        var lastQuadControl: CGPoint?
        var command: Character = " "

        while let next = scanner.nextCommandOrNumber() {
            switch next {
            case .command(let c):
                command = c
            case .number(let n):
                scanner.pushBack(n)
            }
            guard let op = Op(command) else { continue }
            let relative = command.isLowercase

            func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
                relative ? CGPoint(x: current.x + x, y: current.y + y) : CGPoint(x: x, y: y)
            }

            switch op {
            case .move:
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                current = pt(x, y)
                start = current
                path.move(to: current)
                // Subsequent pairs are implicit linetos.
                command = relative ? "l" : "L"
                lastCubicControl = nil; lastQuadControl = nil
            case .line:
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                current = pt(x, y)
                path.addLine(to: current)
                lastCubicControl = nil; lastQuadControl = nil
            case .horizontal:
                guard let x = scanner.number() else { return path }
                current = CGPoint(x: relative ? current.x + x : x, y: current.y)
                path.addLine(to: current)
                lastCubicControl = nil; lastQuadControl = nil
            case .vertical:
                guard let y = scanner.number() else { return path }
                current = CGPoint(x: current.x, y: relative ? current.y + y : y)
                path.addLine(to: current)
                lastCubicControl = nil; lastQuadControl = nil
            case .cubic:
                guard let x1 = scanner.number(), let y1 = scanner.number(),
                      let x2 = scanner.number(), let y2 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c1 = pt(x1, y1), c2 = pt(x2, y2), end = pt(x, y)
                path.addCurve(to: end, control1: c1, control2: c2)
                lastCubicControl = c2
                current = end
                lastQuadControl = nil
            case .smoothCubic:
                guard let x2 = scanner.number(), let y2 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c1 = lastCubicControl.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                let c2 = pt(x2, y2), end = pt(x, y)
                path.addCurve(to: end, control1: c1, control2: c2)
                lastCubicControl = c2
                current = end
                lastQuadControl = nil
            case .quad:
                guard let x1 = scanner.number(), let y1 = scanner.number(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let c = pt(x1, y1), end = pt(x, y)
                path.addQuadCurve(to: end, control: c)
                lastQuadControl = c
                current = end
                lastCubicControl = nil
            case .smoothQuad:
                guard let x = scanner.number(), let y = scanner.number() else { return path }
                let c = lastQuadControl.map { CGPoint(x: 2 * current.x - $0.x, y: 2 * current.y - $0.y) } ?? current
                let end = pt(x, y)
                path.addQuadCurve(to: end, control: c)
                lastQuadControl = c
                current = end
                lastCubicControl = nil
            case .arc:
                guard let rx = scanner.number(), let ry = scanner.number(),
                      let rot = scanner.number(), let largeArc = scanner.flag(),
                      let sweep = scanner.flag(),
                      let x = scanner.number(), let y = scanner.number() else { return path }
                let end = pt(x, y)
                addArc(&path, from: current, to: end, rx: rx, ry: ry,
                       rotationDeg: rot, largeArc: largeArc, sweep: sweep)
                current = end
                lastCubicControl = nil; lastQuadControl = nil
            case .close:
                path.closeSubpath()
                current = start
                lastCubicControl = nil; lastQuadControl = nil
            }
        }
        return path
    }

    private enum Op {
        case move, line, horizontal, vertical, cubic, smoothCubic, quad, smoothQuad, arc, close
        init?(_ c: Character) {
            switch Character(c.lowercased()) {
            case "m": self = .move
            case "l": self = .line
            case "h": self = .horizontal
            case "v": self = .vertical
            case "c": self = .cubic
            case "s": self = .smoothCubic
            case "q": self = .quad
            case "t": self = .smoothQuad
            case "a": self = .arc
            case "z": self = .close
            default: return nil
            }
        }
    }

    /// Endpoint→center arc conversion (SVG 1.1 F.6), emitted as cubic segments.
    private static func addArc(_ path: inout Path, from p0: CGPoint, to p1: CGPoint,
                               rx rxIn: CGFloat, ry ryIn: CGFloat,
                               rotationDeg: CGFloat, largeArc: Bool, sweep: Bool) {
        var rx = abs(rxIn), ry = abs(ryIn)
        if rx == 0 || ry == 0 || p0 == p1 {
            path.addLine(to: p1)
            return
        }
        let phi = rotationDeg * .pi / 180
        let dx2 = (p0.x - p1.x) / 2, dy2 = (p0.y - p1.y) / 2
        let x1p = cos(phi) * dx2 + sin(phi) * dy2
        let y1p = -sin(phi) * dx2 + cos(phi) * dy2
        let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
        if lambda > 1 {
            let s = sqrt(lambda)
            rx *= s; ry *= s
        }
        let sign: CGFloat = largeArc != sweep ? 1 : -1
        let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p
        let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p
        let coeff = sign * sqrt(max(0, num / den))
        let cxp = coeff * (rx * y1p) / ry
        let cyp = coeff * -(ry * x1p) / rx
        let cx = cos(phi) * cxp - sin(phi) * cyp + (p0.x + p1.x) / 2
        let cy = sin(phi) * cxp + cos(phi) * cyp + (p0.y + p1.y) / 2

        func angle(_ ux: CGFloat, _ uy: CGFloat, _ vx: CGFloat, _ vy: CGFloat) -> CGFloat {
            let dot = ux * vx + uy * vy
            let len = sqrt((ux * ux + uy * uy) * (vx * vx + vy * vy))
            var ang = acos(min(max(dot / len, -1), 1))
            if ux * vy - uy * vx < 0 { ang = -ang }
            return ang
        }
        let theta1 = angle(1, 0, (x1p - cxp) / rx, (y1p - cyp) / ry)
        var delta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry)
        if !sweep, delta > 0 { delta -= 2 * .pi }
        if sweep, delta < 0 { delta += 2 * .pi }

        // Split into <= 90° cubic segments.
        let segments = max(1, Int(ceil(abs(delta) / (.pi / 2))))
        let segDelta = delta / CGFloat(segments)
        let t = 4 / 3 * tan(segDelta / 4)
        var theta = theta1
        var from = p0
        for _ in 0..<segments {
            let cosT = cos(theta), sinT = sin(theta)
            let theta2 = theta + segDelta
            let cosT2 = cos(theta2), sinT2 = sin(theta2)

            func point(_ c: CGFloat, _ s: CGFloat) -> CGPoint {
                CGPoint(x: cx + rx * (cos(phi) * c) - ry * (sin(phi) * s),
                        y: cy + rx * (sin(phi) * c) + ry * (cos(phi) * s))
            }
            func deriv(_ c: CGFloat, _ s: CGFloat) -> CGPoint {
                CGPoint(x: -rx * cos(phi) * s - ry * sin(phi) * c,
                        y: -rx * sin(phi) * s + ry * cos(phi) * c)
            }
            let to = point(cosT2, sinT2)
            let d1 = deriv(cosT, sinT)
            let d2 = deriv(cosT2, sinT2)
            let c1 = CGPoint(x: from.x + t * d1.x, y: from.y + t * d1.y)
            let c2 = CGPoint(x: to.x - t * d2.x, y: to.y - t * d2.y)
            path.addCurve(to: to, control1: c1, control2: c2)
            from = to
            theta = theta2
        }
    }

    private struct Tokenizer {
        enum Token {
            case command(Character)
            case number(CGFloat)
        }

        private let chars: [Character]
        private var index = 0
        private var pushed: CGFloat?

        init(_ s: String) {
            chars = Array(s)
        }

        mutating func pushBack(_ n: CGFloat) {
            pushed = n
        }

        mutating func nextCommandOrNumber() -> Token? {
            if let p = pushed {
                pushed = nil
                return .number(p)
            }
            skipSeparators()
            guard index < chars.count else { return nil }
            let c = chars[index]
            if c.isLetter {
                index += 1
                return .command(c)
            }
            return number().map { .number($0) }
        }

        mutating func number() -> CGFloat? {
            if let p = pushed {
                pushed = nil
                return p
            }
            skipSeparators()
            guard index < chars.count else { return nil }
            var s = ""
            if chars[index] == "-" || chars[index] == "+" {
                s.append(chars[index]); index += 1
            }
            var seenDot = false
            while index < chars.count {
                let c = chars[index]
                if c.isNumber {
                    s.append(c); index += 1
                } else if c == "." {
                    if seenDot { break }  // "1.5.5" = two numbers
                    seenDot = true
                    s.append(c); index += 1
                } else if c == "e" || c == "E" {
                    s.append(c); index += 1
                    if index < chars.count, chars[index] == "-" || chars[index] == "+" {
                        s.append(chars[index]); index += 1
                    }
                } else {
                    break
                }
            }
            return s.isEmpty || s == "-" || s == "+" ? nil : Double(s).map { CGFloat($0) }
        }

        /// Arc flags are single chars, possibly unseparated ("00 1").
        mutating func flag() -> Bool? {
            skipSeparators()
            guard index < chars.count, chars[index] == "0" || chars[index] == "1" else { return nil }
            defer { index += 1 }
            return chars[index] == "1"
        }

        private mutating func skipSeparators() {
            while index < chars.count, chars[index] == "," || chars[index].isWhitespace {
                index += 1
            }
        }
    }
}

/// Who makes a model, read from its id; a gateway like opencode's Zen serves
/// many vendors under one provider. Case order is the picker's section order.
enum ModelVendor: CaseIterable {
    case anthropic, openai, google, xai, deepseek, qwen, moonshot, zai, minimax, mistral, meta, nvidia, xiaomi, meituan, other

    var title: String {
        switch self {
        case .anthropic: return "Anthropic"
        case .openai: return "OpenAI"
        case .google: return "Google"
        case .xai: return "xAI"
        case .deepseek: return "DeepSeek"
        case .qwen: return "Qwen"
        case .moonshot: return "Moonshot AI"
        case .zai: return "Z.ai"
        case .minimax: return "MiniMax"
        case .mistral: return "Mistral"
        case .meta: return "Meta"
        case .nvidia: return "NVIDIA"
        case .xiaomi: return "Xiaomi"
        case .meituan: return "Meituan"
        case .other: return "Other"
        }
    }

    /// Nil for `.other`, which falls back to the harness mark.
    var mark: BrandMark? {
        switch self {
        case .anthropic: return .claude
        case .openai: return .openai
        case .google: return .gemini
        case .xai: return .grok
        case .deepseek: return .deepseek
        case .qwen: return .qwen
        case .moonshot: return .kimi
        case .zai: return .zai
        case .minimax: return .minimax
        case .mistral: return .mistral
        case .meta: return .meta
        case .nvidia: return .nvidia
        case .xiaomi: return .xiaomi
        case .meituan: return .longcat
        case .other: return nil
        }
    }

    private var prefixes: [String] {
        switch self {
        case .anthropic: return ["claude"]
        case .openai: return ["gpt", "codex", "o1", "o3", "o4", "chatgpt"]
        case .google: return ["gemini", "gemma"]
        case .xai: return ["grok"]
        case .deepseek: return ["deepseek"]
        case .qwen: return ["qwen", "qwq"]
        case .moonshot: return ["kimi", "moonshot"]
        case .zai: return ["glm"]
        case .minimax: return ["minimax"]
        case .mistral: return ["mistral", "codestral", "devstral", "magistral", "ministral"]
        case .meta: return ["llama", "muse"]
        case .nvidia: return ["nemotron"]
        case .xiaomi: return ["mimo"]
        case .meituan: return ["longcat"]
        case .other: return []
        }
    }

    static func of(_ modelId: String) -> ModelVendor {
        let name = (modelId.split(separator: "/").last.map(String.init) ?? modelId).lowercased()
        return allCases.first { v in v.prefixes.contains { name.hasPrefix($0) } } ?? .other
    }
}

/// Models grouped by vendor in picker order; nil when they're all one
/// vendor's, so short lists stay flat.
func vendorGroups<T>(_ models: [T], id: (T) -> String) -> [(vendor: ModelVendor, models: [T])]? {
    let groups = Dictionary(grouping: models) { ModelVendor.of(id($0)) }
    guard groups.count > 1 else { return nil }
    return ModelVendor.allCases.compactMap { v in groups[v].map { (v, $0) } }
}

/// Brand marks rendered once per (harness, size) into cached images, in their
/// brand color; black-or-white marks are templates tinted by the caller.
enum BrandMarks {
    private static var cache: [String: UIImage] = [:]

    static func image(for harness: String?, side: CGFloat = 15) -> UIImage? {
        guard let harness, !harness.isEmpty else { return nil }
        let mark = BrandMark.forHarness(harness)
        return image(mark, tint: mark.color, key: "\(harness)@\(side)", side: side)
    }

    /// The mark for a chosen model: its vendor's, else its harness's.
    static func image(forModel model: String?, harness: String?, side: CGFloat = 15) -> UIImage? {
        image(for: model.map(ModelVendor.of) ?? .other, harness: harness, side: side)
    }

    /// A model vendor's mark; `.other` falls back to the harness mark.
    static func image(for vendor: ModelVendor, harness: String?, side: CGFloat = 15) -> UIImage? {
        guard let mark = vendor.mark else { return image(for: harness, side: side) }
        return image(mark, tint: mark.color, key: "vendor:\(vendor)@\(side)", side: side)
    }

    private static func image(_ mark: BrandMark, tint: UIColor?, key: String, side: CGFloat) -> UIImage? {
        if let hit = cache[key] { return hit }
        let rect = CGRect(x: 0, y: 0, width: side, height: side)
        let path = BrandMarkShape(mark: mark).path(in: rect).cgPath
        let image = UIGraphicsImageRenderer(size: rect.size).image { ctx in
            ctx.cgContext.addPath(path)
            ctx.cgContext.setFillColor((tint ?? .black).cgColor)
            ctx.cgContext.fillPath(using: mark.evenOddFill ? .evenOdd : .winding)
        }
        let result = tint == nil ? image.withRenderingMode(.alwaysTemplate) : image.withRenderingMode(.alwaysOriginal)
        cache[key] = result
        return result
    }
}
