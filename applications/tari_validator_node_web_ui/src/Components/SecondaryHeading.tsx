import Typography from '@mui/material/styles/createTypography';
import theme from '../theme';

interface Props {
  children: string;
}

function SecondaryHeading({ children }: Props) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        width: '100%',
        marginTop: '30px',
      }}
    >
      <h1>{children}</h1>
      <div
        style={{
          background: theme.palette.primary.main,
          width: '100px',
          height: '3px',
        }}
      ></div>
    </div>
  );
}

export default SecondaryHeading;
