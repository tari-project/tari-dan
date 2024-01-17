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

import React from "react";

interface LogoProps {
  fill?: string;
}

const TariLogo: React.FC<LogoProps> = ({ fill = '"#000000' }) => (
  <svg xmlns="http://www.w3.org/2000/svg" width="130" height="39" viewBox="0 0 130 39" fill="none">
    <path
      d="M81.0311 21.4987L83.4846 13.4419L85.9382 21.4987H81.0311ZM80.8368 5.7793L71.6851 32.812H77.5392L79.4711 26.5619H87.4982L89.4301 32.812H95.2842L86.1318 5.7793H80.8368Z"
      fill={fill}
    />
    <path d="M123.72 5.57129H129.161V32.604H123.72V5.57129Z" fill={fill} />
    <path
      d="M103.82 17.3697V10.8425H108.447C110.558 10.8425 111.673 11.971 111.673 14.1064C111.673 16.2414 110.558 17.3697 108.447 17.3697H103.82ZM109.326 22.4078C114.324 22.1458 117.19 19.1201 117.19 14.1064C117.19 8.89224 113.992 5.7793 108.637 5.7793H98.3785V32.8118H103.82V24.364L111.272 32.8118H118.28L109.032 22.4233L109.326 22.4078Z"
      fill={fill}
    />
    <path d="M58.3158 32.812H63.7572V10.8427H72.8918V5.7793H49.1806V10.8427H58.3158V32.812Z" fill={fill} />
    <path
      d="M35.3043 12.8503L35.2998 17.537L7.77942 10.4572L17.0735 4.39937L35.3043 12.8503ZM18.6108 32.5679L18.6037 17.2241L33.2819 21.0046L18.6108 32.5679ZM14.7696 31.0976L4.679 19.7853L4.67302 13.6107L14.7536 16.2324L14.7696 31.0976ZM0.838745 10.4037L0.840223 21.265L16.6841 39L39.1108 21.3118L39.1423 10.3873L16.7354 0L0.838745 10.4037Z"
      fill={fill}
    />
  </svg>
);

export default TariLogo;
