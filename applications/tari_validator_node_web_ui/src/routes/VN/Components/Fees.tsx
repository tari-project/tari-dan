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

import React, { useCallback, useState } from 'react';
import { getFees } from '../../../utils/json_rpc';
import Table from '@mui/material/Table';
import TableBody from '@mui/material/TableBody';
import TableCell from '@mui/material/TableCell';
import TableContainer from '@mui/material/TableContainer';
import TableHead from '@mui/material/TableHead';
import TableRow from '@mui/material/TableRow';
import { DataTableCell } from '../../../Components/StyledComponents';
import Button from '@mui/material/Button';
import { TextField } from '@mui/material';
import { Form } from 'react-router-dom';
import Fade from '@mui/material/Fade';
import Divider from '@mui/material/Divider';

interface IFees {
    epoch: number;
    claimablePublicKey: string;
    totalAccruedFee: number;
}

function Fees() {
    const [fees, setFees] = useState<IFees>({
        epoch: 0,
        claimablePublicKey: '',
        totalAccruedFee: 0,
    });
    const [showFees, setShowFees] = useState(false);
    const [formState, setFormState] = useState({ epoch: '', publicKey: '' });

    let fetchFees = useCallback(async () => {
        const resp = await getFees(parseInt(formState.epoch), formState.publicKey);
        setFees({
            epoch: parseInt(formState.epoch),
            claimablePublicKey: formState.publicKey,
            totalAccruedFee: resp.totalAccruedFee,
        });
    }, []);

    // fetchFees should actually be called here, but I don't have access to the method on the mock server to test it
    // so you can delete the setFees call underneath, uncomment the fetchFees method and then try connecting to the
    // VN for real data for totalAccruedFee
    // if we want to later display various searches, we can just change fees to an array of objects and then map over it
    const getVNFees = () => {
        fetchFees();
        setShowFees(true);
    };

    const onChange = (e: React.ChangeEvent<HTMLInputElement>) => {
        e.preventDefault();
        setFormState({ ...formState, [e.target.name]: e.target.value });
    };

    const onCancel = () => {
        setFormState({ epoch: '', publicKey: '' });
        setShowFees(false);
    };

    return (
        <>
            <Form onSubmit={getVNFees} className="flex-container">
                <TextField
                    name="epoch"
                    label="Epoch"
                    value={formState.epoch}
                    onChange={onChange}
                    style={{ flexGrow: 1 }}
                />
                <TextField
                    name="publicKey"
                    label="VN Public Key"
                    value={formState.publicKey}
                    onChange={onChange}
                    style={{ flexGrow: 10 }}
                />
                <>
                    <Button variant="contained" type="submit">
                        Calculate Fees
                    </Button>
                    <Button variant="outlined" onClick={onCancel}>
                        Clear
                    </Button>
                </>
            </Form>
            {showFees && (
                <Fade in={showFees}>
                    <div>
                        <Divider style={{ marginBottom: '10px', marginTop: '20px' }} />
                        <TableContainer>
                            <Table>
                                <TableRow>
                                    <TableCell width={200}>Epoch</TableCell>
                                    <DataTableCell>{fees.epoch}</DataTableCell>
                                </TableRow>
                                <TableRow>
                                    <TableCell>Public Key</TableCell>
                                    <DataTableCell>{fees.claimablePublicKey}</DataTableCell>
                                </TableRow>
                                <TableRow>
                                    <TableCell>Fees</TableCell>
                                    <DataTableCell>{fees.totalAccruedFee} Tari</DataTableCell>
                                </TableRow>
                            </Table>
                        </TableContainer>
                    </div>
                </Fade>
            )}
        </>
    );
}

export default Fees;
