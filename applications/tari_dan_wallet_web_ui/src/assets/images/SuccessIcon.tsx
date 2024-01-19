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

const SuccessIcon: React.FC = () => {
  return (
    <svg width="100" height="101" viewBox="0 0 100 101" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path
        d="M50 18C32.0797 18 17.5 32.5797 17.5 50.5C17.5 68.4203 32.0797 83 50 83C67.9203 83 82.5 68.4203 82.5 50.5C82.5 32.5797 67.9203 18 50 18ZM66.9141 39.6078L45.9141 64.6078C45.6837 64.8822 45.3971 65.1039 45.0736 65.2578C44.7501 65.4117 44.3973 65.4943 44.0391 65.5H43.9969C43.6465 65.4999 43.3 65.4261 42.9799 65.2834C42.6599 65.1408 42.3734 64.9324 42.1391 64.6719L33.1391 54.6719C32.9105 54.4295 32.7327 54.1438 32.6161 53.8316C32.4995 53.5195 32.4465 53.1872 32.4602 52.8543C32.4738 52.5214 32.5539 52.1946 32.6957 51.8931C32.8375 51.5916 33.0381 51.3214 33.2857 51.0986C33.5334 50.8757 33.8231 50.7046 34.1379 50.5952C34.4526 50.4859 34.786 50.4406 35.1185 50.462C35.451 50.4834 35.7759 50.571 36.0741 50.7198C36.3722 50.8685 36.6376 51.0754 36.8547 51.3281L43.9313 59.1906L63.0859 36.3922C63.5156 35.8954 64.1235 35.5877 64.7782 35.5355C65.4329 35.4834 66.0819 35.6909 66.5848 36.1134C67.0877 36.5358 67.4041 37.1392 67.4657 37.7931C67.5274 38.447 67.3292 39.0989 66.9141 39.6078Z"
        fill="#9330FF"
      />
      <circle opacity="0.4" cx="50" cy="50.5" r="36.5" stroke="#9330FF" />
      <circle opacity="0.1" cx="50" cy="50.5" r="49.5" stroke="#9330FF" />
    </svg>
  );
};

export default SuccessIcon;
