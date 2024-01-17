//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

interface LogoProps {
  width?: string;
  height?: string;
  fill?: string;
}

const Logo: React.FC<LogoProps> = ({ width = "100%", height = "auto", fill = "black" }) => (
  <svg width={width} height={height} viewBox="0 -5 350 60" xmlns="http://www.w3.org/2000/svg">
    <path
      d="M115.149 30.8703L118.672 19.3015L122.195 30.8703H115.149ZM114.87 8.29883L101.729 47.115H110.135L112.909 38.1405H124.435L127.209 47.115H135.615L122.473 8.29883H114.87Z"
      fill={fill}
    />
    <path d="M176.445 8H184.258V46.8162H176.445V8Z" fill={fill} />
    <path
      d="M147.87 24.9415V15.5691H154.515C157.546 15.5691 159.147 17.1895 159.147 20.2557C159.147 23.3213 157.546 24.9415 154.515 24.9415H147.87ZM155.777 32.1757C162.953 31.7994 167.069 27.4548 167.069 20.2557C167.069 12.7687 162.477 8.29883 154.787 8.29883H140.057V47.1147H147.87V34.9845L158.571 47.1147H168.634L155.354 32.1979L155.777 32.1757Z"
      fill={fill}
    />
    <path d="M82.5313 47.115H90.3446V15.5694H103.461V8.29883H69.4141V15.5694H82.5313V47.115Z" fill={fill} />
    <path
      d="M49.489 18.4517L49.4826 25.1813L9.9661 15.0154L23.3114 6.31705L49.489 18.4517ZM25.5188 46.7641L25.5086 24.732L46.5851 30.1605L25.5188 46.7641ZM20.0033 44.6529L5.51421 28.4097L5.50562 19.5436L19.9803 23.3081L20.0033 44.6529ZM0 14.9387L0.00212257 30.5344L22.7523 56L54.9548 30.6016L55 14.9151L22.826 0L0 14.9387Z"
      fill={fill}
    />
    <path
      d="M197.968 29.4329H201.976L210.208 42.0569H210.256V29.4329H213.28V46.4249H209.44L201.04 33.3929H200.992V46.4249H197.968V29.4329Z"
      fill={fill}
    />
    <path
      d="M216.157 38.0009C216.157 36.6409 216.381 35.4089 216.829 34.3049C217.277 33.1849 217.893 32.2329 218.677 31.4489C219.477 30.6649 220.421 30.0649 221.509 29.6489C222.613 29.2169 223.821 29.0009 225.133 29.0009C226.461 28.9849 227.677 29.1849 228.781 29.6009C229.885 30.0009 230.837 30.5929 231.637 31.3769C232.437 32.1609 233.061 33.1049 233.509 34.2089C233.957 35.3129 234.181 36.5449 234.181 37.9049C234.181 39.2329 233.957 40.4409 233.509 41.5289C233.061 42.6169 232.437 43.5529 231.637 44.3369C230.837 45.1209 229.885 45.7369 228.781 46.1849C227.677 46.6169 226.461 46.8409 225.133 46.8569C223.821 46.8569 222.613 46.6489 221.509 46.2329C220.421 45.8009 219.477 45.2009 218.677 44.4329C217.893 43.6489 217.277 42.7129 216.829 41.6249C216.381 40.5369 216.157 39.3289 216.157 38.0009ZM219.325 37.8089C219.325 38.7209 219.461 39.5609 219.733 40.3289C220.021 41.0969 220.421 41.7609 220.933 42.3209C221.445 42.8809 222.053 43.3209 222.757 43.6409C223.477 43.9609 224.277 44.1209 225.157 44.1209C226.037 44.1209 226.837 43.9609 227.557 43.6409C228.277 43.3209 228.893 42.8809 229.405 42.3209C229.917 41.7609 230.309 41.0969 230.581 40.3289C230.869 39.5609 231.013 38.7209 231.013 37.8089C231.013 36.9609 230.869 36.1689 230.581 35.4329C230.309 34.6969 229.917 34.0569 229.405 33.5129C228.893 32.9529 228.277 32.5209 227.557 32.2169C226.837 31.8969 226.037 31.7369 225.157 31.7369C224.277 31.7369 223.477 31.8969 222.757 32.2169C222.053 32.5209 221.445 32.9529 220.933 33.5129C220.421 34.0569 220.021 34.6969 219.733 35.4329C219.461 36.1689 219.325 36.9609 219.325 37.8089Z"
      fill={fill}
    />
    <path
      d="M237.062 29.4329H243.758C244.878 29.4329 245.958 29.6089 246.998 29.9609C248.038 30.2969 248.958 30.8169 249.758 31.5209C250.558 32.2249 251.198 33.1129 251.678 34.1849C252.158 35.2409 252.398 36.4889 252.398 37.9289C252.398 39.3849 252.118 40.6489 251.558 41.7209C251.014 42.7769 250.302 43.6569 249.422 44.3609C248.558 45.0489 247.59 45.5689 246.518 45.9209C245.462 46.2569 244.422 46.4249 243.398 46.4249H237.062V29.4329ZM242.342 43.6889C243.286 43.6889 244.174 43.5849 245.006 43.3769C245.854 43.1529 246.59 42.8169 247.214 42.3689C247.838 41.9049 248.326 41.3129 248.678 40.5929C249.046 39.8569 249.23 38.9689 249.23 37.9289C249.23 36.9049 249.07 36.0249 248.75 35.2889C248.43 34.5529 247.982 33.9609 247.406 33.5129C246.846 33.0489 246.174 32.7129 245.39 32.5049C244.622 32.2809 243.774 32.1689 242.846 32.1689H240.086V43.6889H242.342Z"
      fill={fill}
    />
    <path
      d="M255.272 29.4329H266.528V32.1689H258.296V36.3449H266.096V39.0809H258.296V43.6889H266.96V46.4249H255.272V29.4329Z"
      fill={fill}
    />
    <path
      d="M196 8.43289H199.48L204.112 21.2729L208.888 8.43289H212.152L205.288 25.4249H202.672L196 8.43289Z"
      fill={fill}
    />
    <path
      d="M218.159 8.43289H220.775L228.095 25.4249H224.639L223.055 21.5369H215.687L214.151 25.4249H210.767L218.159 8.43289ZM221.999 18.9449L219.383 12.0329L216.719 18.9449H221.999Z"
      fill={fill}
    />
    <path d="M229.984 8.43289H233.008V22.6889H240.232V25.4249H229.984V8.43289Z" fill={fill} />
    <path d="M242.357 8.43289H245.381V25.4249H242.357V8.43289Z" fill={fill} />
    <path
      d="M249.109 8.43289H255.805C256.925 8.43289 258.005 8.60889 259.045 8.96089C260.085 9.29689 261.005 9.81689 261.805 10.5209C262.605 11.2249 263.245 12.1129 263.725 13.1849C264.205 14.2409 264.445 15.4889 264.445 16.9289C264.445 18.3849 264.165 19.6489 263.605 20.7209C263.061 21.7769 262.349 22.6569 261.469 23.3609C260.605 24.0489 259.637 24.5689 258.565 24.9209C257.509 25.2569 256.469 25.4249 255.445 25.4249H249.109V8.43289ZM254.389 22.6889C255.333 22.6889 256.221 22.5849 257.053 22.3769C257.901 22.1529 258.637 21.8169 259.261 21.3689C259.885 20.9049 260.373 20.3129 260.725 19.5929C261.093 18.8569 261.277 17.9689 261.277 16.9289C261.277 15.9049 261.117 15.0249 260.797 14.2889C260.477 13.5529 260.029 12.9609 259.453 12.5129C258.893 12.0489 258.221 11.7129 257.437 11.5049C256.669 11.2809 255.821 11.1689 254.893 11.1689H252.133V22.6889H254.389Z"
      fill={fill}
    />
    <path
      d="M272.816 8.43289H275.432L282.752 25.4249H279.296L277.712 21.5369H270.344L268.808 25.4249H265.424L272.816 8.43289ZM276.656 18.9449L274.04 12.0329L271.376 18.9449H276.656Z"
      fill={fill}
    />
    <path d="M285.917 11.1689H280.709V8.43289H294.149V11.1689H288.941V25.4249H285.917V11.1689Z" fill={fill} />
    <path
      d="M295.306 17.0009C295.306 15.6409 295.53 14.4089 295.978 13.3049C296.426 12.1849 297.042 11.2329 297.826 10.4489C298.626 9.66489 299.57 9.06489 300.658 8.64889C301.762 8.21689 302.97 8.00089 304.282 8.00089C305.61 7.98489 306.826 8.18489 307.93 8.60089C309.034 9.00089 309.986 9.59289 310.786 10.3769C311.586 11.1609 312.21 12.1049 312.658 13.2089C313.106 14.3129 313.33 15.5449 313.33 16.9049C313.33 18.2329 313.106 19.4409 312.658 20.5289C312.21 21.6169 311.586 22.5529 310.786 23.3369C309.986 24.1209 309.034 24.7369 307.93 25.1849C306.826 25.6169 305.61 25.8409 304.282 25.8569C302.97 25.8569 301.762 25.6489 300.658 25.2329C299.57 24.8009 298.626 24.2009 297.826 23.4329C297.042 22.6489 296.426 21.7129 295.978 20.6249C295.53 19.5369 295.306 18.3289 295.306 17.0009ZM298.474 16.8089C298.474 17.7209 298.61 18.5609 298.882 19.3289C299.17 20.0969 299.57 20.7609 300.082 21.3209C300.594 21.8809 301.202 22.3209 301.906 22.6409C302.626 22.9609 303.426 23.1209 304.306 23.1209C305.186 23.1209 305.986 22.9609 306.706 22.6409C307.426 22.3209 308.042 21.8809 308.554 21.3209C309.066 20.7609 309.458 20.0969 309.73 19.3289C310.018 18.5609 310.162 17.7209 310.162 16.8089C310.162 15.9609 310.018 15.1689 309.73 14.4329C309.458 13.6969 309.066 13.0569 308.554 12.5129C308.042 11.9529 307.426 11.5209 306.706 11.2169C305.986 10.8969 305.186 10.7369 304.306 10.7369C303.426 10.7369 302.626 10.8969 301.906 11.2169C301.202 11.5209 300.594 11.9529 300.082 12.5129C299.57 13.0569 299.17 13.6969 298.882 14.4329C298.61 15.1689 298.474 15.9609 298.474 16.8089Z"
      fill={fill}
    />
    <path
      d="M316.21 8.43289H322.114C322.93 8.43289 323.714 8.51289 324.466 8.67289C325.234 8.81689 325.914 9.07289 326.506 9.44089C327.098 9.80889 327.57 10.3049 327.922 10.9289C328.274 11.5529 328.45 12.3449 328.45 13.3049C328.45 14.5369 328.106 15.5689 327.418 16.4009C326.746 17.2329 325.778 17.7369 324.514 17.9129L329.026 25.4249H325.378L321.442 18.2249H319.234V25.4249H316.21V8.43289ZM321.586 15.6329C322.018 15.6329 322.45 15.6169 322.882 15.5849C323.314 15.5369 323.706 15.4409 324.058 15.2969C324.426 15.1369 324.722 14.9049 324.946 14.6009C325.17 14.2809 325.282 13.8409 325.282 13.2809C325.282 12.7849 325.178 12.3849 324.97 12.0809C324.762 11.7769 324.49 11.5529 324.154 11.4089C323.818 11.2489 323.442 11.1449 323.026 11.0969C322.626 11.0489 322.234 11.0249 321.85 11.0249H319.234V15.6329H321.586Z"
      fill={fill}
    />
  </svg>
);

export default Logo;
